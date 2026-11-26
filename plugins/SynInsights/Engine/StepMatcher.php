<?php

/**
 * Synatomo — analyse produit pour Matomo.
 *
 * @license https://www.gnu.org/licenses/gpl-3.0.html GPL v3 or later
 */

namespace Piwik\Plugins\SynInsights\Engine;

/**
 * Décide si une action enregistrée correspond à une étape d'entonnoir.
 *
 * Une étape est un tableau :
 *   ['name' => 'Panier', 'kind' => 'url'|'title'|'event', 'match' => 'equals'|'contains'|'starts_with'|'ends_with'|'regex',
 *    'pattern' => '/panier', 'category' => '', 'action' => '', 'event_name' => '']
 * Pour `url` et `title`, `match` et `pattern` s'appliquent. Pour `event`, les trois
 * champs category/action/event_name sont comparés à l'identique ; un champ vide accepte tout.
 *
 * La classe ne lit pas la base : on lui fournit le libellé de chaque idaction.
 */
class StepMatcher
{
    public const KINDS = ['url', 'title', 'event'];
    public const MATCHES = ['equals', 'contains', 'starts_with', 'ends_with', 'regex'];

    /** @var array<int,array> */
    private $steps;
    /** @var array<int,string> idaction => libellé (URL sans protocole, titre, nom d'événement…) */
    private $names;
    /** @var array<string,bool> */
    private $memo = [];

    public function __construct(array $steps, array $actionNames)
    {
        $this->steps = array_values($steps);
        $this->names = $actionNames;
    }

    /** Ajoute des libellés d'actions découverts en cours de lecture. */
    public function addNames(array $actionNames): void
    {
        $this->names += $actionNames;
    }

    public function count(): int
    {
        return count($this->steps);
    }

    /**
     * @param array $row ligne de log_link_visit_action (idaction_url, idaction_name, idaction_event_category, idaction_event_action)
     */
    public function matches(int $stepIndex, array $row): bool
    {
        $step = $this->steps[$stepIndex];
        $isEvent = !empty($row['idaction_event_category']) || !empty($row['idaction_event_action']);

        if ($step['kind'] === 'event') {
            if (!$isEvent) {
                return false;
            }
            return $this->sameLabel($step['category'] ?? '', $row['idaction_event_category'] ?? null)
                && $this->sameLabel($step['action'] ?? '', $row['idaction_event_action'] ?? null)
                && $this->sameLabel($step['event_name'] ?? '', $row['idaction_name'] ?? null);
        }

        if ($isEvent) {
            return false; // une étape de page ne se valide pas sur un événement
        }
        $id = (int) ($step['kind'] === 'url' ? ($row['idaction_url'] ?? 0) : ($row['idaction_name'] ?? 0));
        if ($id === 0) {
            return false;
        }
        $key = $stepIndex . ':' . $id;
        if (!isset($this->memo[$key])) {
            $label = $this->names[$id] ?? null;
            $this->memo[$key] = $label !== null && self::test($step, $label);
        }
        return $this->memo[$key];
    }

    private function sameLabel(string $expected, $idaction): bool
    {
        if ($expected === '') {
            return true;
        }
        return $idaction !== null && ($this->names[(int) $idaction] ?? null) === $expected;
    }

    /**
     * Compare un motif à un libellé. Pour une URL, le motif peut être écrit avec ou sans
     * protocole, avec ou sans nom d'hôte : « /panier », « demo.test/panier » et
     * « https://demo.test/panier » désignent la même page.
     */
    public static function test(array $step, string $label): bool
    {
        $pattern = (string) ($step['pattern'] ?? '');
        $candidates = [$label];
        if ($step['kind'] === 'url') {
            $pattern = preg_replace('#^https?://#i', '', $pattern);
            $candidates[] = self::pathOf($label);
        }
        foreach ($candidates as $value) {
            if (self::compare($step['match'] ?? 'equals', $pattern, $value)) {
                return true;
            }
        }
        return false;
    }

    /** « demo.test/produits?x=1 » → « /produits?x=1 » */
    public static function pathOf(string $urlWithoutScheme): string
    {
        $slash = strpos($urlWithoutScheme, '/');
        return $slash === false ? '/' : substr($urlWithoutScheme, $slash);
    }

    private static function compare(string $match, string $pattern, string $value): bool
    {
        switch ($match) {
            case 'contains':
                return $pattern !== '' && stripos($value, $pattern) !== false;
            case 'starts_with':
                return $pattern !== '' && stripos($value, $pattern) === 0;
            case 'ends_with':
                return $pattern !== '' && strcasecmp(substr($value, -strlen($pattern)), $pattern) === 0;
            case 'regex':
                // Le motif vient d'un utilisateur : délimiteur neutre, erreurs de syntaxe traitées comme « ne correspond pas ».
                $regex = '~' . str_replace('~', '\~', $pattern) . '~iu';
                return @preg_match($regex, $value) === 1;
            case 'equals':
            default:
                return strcasecmp(rtrim($value, '/'), rtrim($pattern, '/')) === 0;
        }
    }

    /**
     * Valide et normalise une liste d'étapes saisie par l'utilisateur.
     *
     * @throws \InvalidArgumentException message destiné à l'utilisateur
     */
    public static function sanitizeSteps($steps): array
    {
        if (!is_array($steps) || count($steps) < 2) {
            throw new \InvalidArgumentException('Un entonnoir compte au moins deux étapes.');
        }
        if (count($steps) > 12) {
            throw new \InvalidArgumentException('Un entonnoir compte au plus douze étapes.');
        }
        $clean = [];
        foreach (array_values($steps) as $i => $step) {
            $n = $i + 1;
            if (!is_array($step)) {
                throw new \InvalidArgumentException("Étape $n : format invalide.");
            }
            $kind = (string) ($step['kind'] ?? 'url');
            if (!in_array($kind, self::KINDS, true)) {
                throw new \InvalidArgumentException("Étape $n : type inconnu.");
            }
            $item = ['name' => mb_substr(trim((string) ($step['name'] ?? '')), 0, 80), 'kind' => $kind];
            if ($kind === 'event') {
                foreach (['category', 'action', 'event_name'] as $field) {
                    $item[$field] = mb_substr(trim((string) ($step[$field] ?? '')), 0, 200);
                }
                if ($item['category'] === '' && $item['action'] === '' && $item['event_name'] === '') {
                    throw new \InvalidArgumentException("Étape $n : indiquez au moins la catégorie, l'action ou le nom de l'événement.");
                }
                $fallback = implode(' › ', array_filter([$item['category'], $item['action'], $item['event_name']], 'strlen'));
            } else {
                $match = (string) ($step['match'] ?? 'equals');
                if (!in_array($match, self::MATCHES, true)) {
                    throw new \InvalidArgumentException("Étape $n : comparaison inconnue.");
                }
                $pattern = mb_substr(trim((string) ($step['pattern'] ?? '')), 0, 500);
                if ($pattern === '') {
                    throw new \InvalidArgumentException("Étape $n : le motif est vide.");
                }
                if ($match === 'regex' && @preg_match('~' . str_replace('~', '\~', $pattern) . '~iu', '') === false) {
                    throw new \InvalidArgumentException("Étape $n : expression régulière invalide.");
                }
                $item['match'] = $match;
                $item['pattern'] = $pattern;
                $fallback = $pattern;
            }
            if ($item['name'] === '') {
                $item['name'] = mb_substr($fallback, 0, 80);
            }
            $clean[] = $item;
        }
        return $clean;
    }
}
