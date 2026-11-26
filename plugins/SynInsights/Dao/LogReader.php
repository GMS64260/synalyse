<?php

/**
 * Synatomo — analyse produit pour Matomo.
 *
 * @license https://www.gnu.org/licenses/gpl-3.0.html GPL v3 or later
 */

namespace Piwik\Plugins\SynInsights\Dao;

use Piwik\Common;
use Piwik\Date;
use Piwik\Db;
use Piwik\Period\Factory as PeriodFactory;
use Piwik\Segment;
use Piwik\Site;

/**
 * Lit les journaux de Matomo (log_visit, log_link_visit_action, log_action) pour une
 * période et un segment. Une visite appartient à la période si elle y a commencé.
 *
 * La lecture se fait par paquets de visites : la mémoire reste bornée quel que soit
 * le nombre d'actions.
 */
class LogReader
{
    /** Au-delà, le rapport demande de réduire la période plutôt que de saturer le serveur. */
    public const MAX_VISITS = 200000;
    private const CHUNK = 1500;

    private $idSite;
    private $segment;
    /** @var Date */
    private $start;
    /** @var Date */
    private $end;
    /** @var array<int,string> */
    private $names = [];

    public function __construct(int $idSite, string $period, string $date, string $segment = '')
    {
        $this->idSite = $idSite;
        $this->segment = $segment;
        $timezone = Site::getTimezoneFor($idSite);
        $oPeriod = PeriodFactory::makePeriodFromQueryParams($timezone, $period, $date);
        // Bornes du jour dans le fuseau du site, converties en UTC (les journaux sont en UTC).
        // (Date::factory($chaine, $fuseau) convertit dans l'autre sens : ne pas l'employer ici.)
        $this->start = Date::factory($oPeriod->getDateStart()->setTimezone($timezone)->getDateStartUTC());
        $this->end = Date::factory($oPeriod->getDateEnd()->setTimezone($timezone)->getDateEndUTC());
    }

    /** @return array{0:string,1:string} bornes UTC « AAAA-MM-JJ HH:MM:SS » */
    public function bounds(): array
    {
        return [$this->start->getDatetime(), $this->end->getDatetime()];
    }

    /** @return int[] identifiants des visites de la période, triés */
    public function visitIds(): array
    {
        [$from, $to] = $this->bounds();
        $segment = new Segment($this->segment, [$this->idSite], $this->start, $this->end);
        $query = $segment->getSelectQuery(
            'log_visit.idvisit',
            'log_visit',
            'log_visit.idsite = ? AND log_visit.visit_first_action_time >= ? AND log_visit.visit_first_action_time <= ?',
            [$this->idSite, $from, $to],
            false,
            false,
            self::MAX_VISITS + 1
        );
        $ids = [];
        foreach (Db::getReader()->fetchAll($query['sql'], $query['bind']) as $row) {
            $ids[(int) $row['idvisit']] = true;
        }
        if (count($ids) > self::MAX_VISITS) {
            throw new \RuntimeException('SynInsights_TooManyVisits');
        }
        $ids = array_keys($ids);
        sort($ids);
        return $ids;
    }

    /**
     * Appelle $callback pour chaque action, visite par visite et dans l'ordre chronologique.
     * La ligne contient : idvisit, ts (Unix UTC), idaction_url, idaction_name,
     * idaction_event_category, idaction_event_action.
     * $onNames, s'il est fourni, reçoit avant chaque paquet les libellés d'actions nouvellement connus.
     *
     * @return int nombre de visites lues
     */
    public function eachAction(callable $callback, ?callable $onNames = null): int
    {
        $ids = $this->visitIds();
        $table = Common::prefixTable('log_link_visit_action');
        $db = Db::getReader();
        foreach (array_chunk($ids, self::CHUNK) as $chunk) {
            $rows = $db->fetchAll(
                "SELECT idvisit, idaction_url, idaction_name, idaction_event_category, idaction_event_action, server_time
                   FROM $table
                  WHERE idvisit IN (" . implode(',', array_map('intval', $chunk)) . ')
                  ORDER BY idvisit, server_time, idlink_va'
            );
            $fresh = $this->loadNames($rows);
            if ($onNames && $fresh) {
                $onNames($fresh);
            }
            foreach ($rows as $row) {
                $row['ts'] = strtotime($row['server_time'] . ' UTC');
                $callback($row);
            }
        }
        return count($ids);
    }

    /** Libellé connu d'une action (URL sans protocole, titre, nom d'événement), ou null. */
    public function nameOf(int $idAction): ?string
    {
        return $this->names[$idAction] ?? null;
    }

    private function loadNames(array $rows): array
    {
        $missing = [];
        foreach ($rows as $row) {
            foreach (['idaction_url', 'idaction_name', 'idaction_event_category', 'idaction_event_action'] as $col) {
                $id = (int) $row[$col];
                if ($id && !isset($this->names[$id])) {
                    $missing[$id] = true;
                }
            }
        }
        if (!$missing) {
            return [];
        }
        $fresh = [];
        $table = Common::prefixTable('log_action');
        foreach (array_chunk(array_keys($missing), 2000) as $chunk) {
            foreach (Db::getReader()->fetchAll("SELECT idaction, name FROM $table WHERE idaction IN (" . implode(',', $chunk) . ')') as $row) {
                $fresh[(int) $row['idaction']] = (string) $row['name'];
            }
        }
        $this->names += $fresh;
        return $fresh;
    }

    /**
     * Pages, titres et événements les plus fréquents de la période, pour aider à saisir une étape.
     *
     * @return array{urls:string[], titles:string[], events:array<int,array{category:string,action:string}>}
     */
    public function suggestions(int $limit = 60): array
    {
        [$from, $to] = $this->bounds();
        $lva = Common::prefixTable('log_link_visit_action');
        $action = Common::prefixTable('log_action');
        $db = Db::getReader();
        $bind = [$this->idSite, $from, $to];
        $limit = max(1, min(200, $limit));

        $top = function (string $column) use ($db, $lva, $action, $bind, $limit) {
            return array_column($db->fetchAll(
                "SELECT a.name, COUNT(*) AS n
                   FROM $lva l INNER JOIN $action a ON a.idaction = l.$column
                  WHERE l.idsite = ? AND l.server_time >= ? AND l.server_time <= ? AND l.idaction_event_category IS NULL
                  GROUP BY a.idaction ORDER BY n DESC LIMIT $limit",
                $bind
            ), 'name');
        };
        $events = $db->fetchAll(
            "SELECT c.name AS category, a.name AS action, COUNT(*) AS n
               FROM $lva l
              INNER JOIN $action c ON c.idaction = l.idaction_event_category
              INNER JOIN $action a ON a.idaction = l.idaction_event_action
              WHERE l.idsite = ? AND l.server_time >= ? AND l.server_time <= ?
              GROUP BY c.idaction, a.idaction ORDER BY n DESC LIMIT $limit",
            $bind
        );
        return [
            'urls' => $top('idaction_url'),
            'titles' => $top('idaction_name'),
            'events' => array_map(function ($e) {
                return ['category' => $e['category'], 'action' => $e['action']];
            }, $events),
        ];
    }
}
