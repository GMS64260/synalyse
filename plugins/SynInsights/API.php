<?php

/**
 * Synatomo — analyse produit pour Matomo.
 *
 * @license https://www.gnu.org/licenses/gpl-3.0.html GPL v3 or later
 */

namespace Piwik\Plugins\SynInsights;

use Piwik\Common;
use Piwik\Piwik;
use Piwik\Plugins\SynInsights\Dao\FunnelStore;
use Piwik\Plugins\SynInsights\Dao\LogReader;
use Piwik\Plugins\SynInsights\Engine\FunnelCalculator;
use Piwik\Plugins\SynInsights\Engine\PathCalculator;
use Piwik\Plugins\SynInsights\Engine\StepMatcher;

/**
 * Entonnoirs et parcours des visiteurs, calculés à la demande sur les visites enregistrées.
 *
 * @method static \Piwik\Plugins\SynInsights\API getInstance()
 */
class API extends \Piwik\Plugin\API
{
    /** Entonnoirs enregistrés pour un site. */
    public function getFunnels($idSite)
    {
        Piwik::checkUserHasViewAccess($idSite);
        return (new FunnelStore())->all((int) $idSite);
    }

    /**
     * Crée un entonnoir (idFunnel = 0) ou le modifie.
     *
     * @param string $steps tableau JSON d'étapes, voir Engine\StepMatcher
     * @return array l'entonnoir enregistré
     */
    public function saveFunnel($idSite, $name, $steps, $idFunnel = 0)
    {
        Piwik::checkUserHasWriteAccess($idSite);
        $idSite = (int) $idSite;
        $name = mb_substr(trim(Common::unsanitizeInputValue((string) $name)), 0, 100);
        if ($name === '') {
            throw new \Exception(Piwik::translate('SynInsights_ErrorNameRequired'));
        }
        $clean = $this->decodeSteps($steps);
        $store = new FunnelStore();
        $idFunnel = (int) $idFunnel;
        if ($idFunnel > 0) {
            if (!$store->get($idSite, $idFunnel)) {
                throw new \Exception(Piwik::translate('SynInsights_ErrorFunnelNotFound'));
            }
            $store->update($idSite, $idFunnel, $name, $clean);
        } else {
            $idFunnel = $store->create($idSite, $name, $clean);
        }
        return $store->get($idSite, $idFunnel);
    }

    public function deleteFunnel($idSite, $idFunnel)
    {
        Piwik::checkUserHasWriteAccess($idSite);
        (new FunnelStore())->delete((int) $idSite, (int) $idFunnel);
        return ['deleted' => (int) $idFunnel];
    }

    /**
     * Calcule un entonnoir sur une période : soit un entonnoir enregistré (idFunnel),
     * soit des étapes fournies à la volée (steps, JSON) pour essayer avant d'enregistrer.
     */
    public function getFunnelReport($idSite, $period, $date, $segment = false, $idFunnel = 0, $steps = '')
    {
        Piwik::checkUserHasViewAccess($idSite);
        $idSite = (int) $idSite;
        $name = '';
        if ((int) $idFunnel > 0) {
            $funnel = (new FunnelStore())->get($idSite, (int) $idFunnel);
            if (!$funnel) {
                throw new \Exception(Piwik::translate('SynInsights_ErrorFunnelNotFound'));
            }
            $stepList = $funnel['steps'];
            $name = $funnel['name'];
        } else {
            $stepList = $this->decodeSteps($steps);
        }

        $reader = $this->reader($idSite, $period, $date, $segment);
        $matcher = new StepMatcher($stepList, []);
        $calculator = new FunnelCalculator($matcher);
        try {
            $reader->eachAction([$calculator, 'push'], [$matcher, 'addNames']);
        } catch (\RuntimeException $e) {
            throw new \Exception(Piwik::translate($e->getMessage()));
        }
        $result = $calculator->result();

        $entered = $result['steps'][0]['reached'];
        $out = [];
        foreach ($result['steps'] as $i => $step) {
            $next = [];
            foreach (array_slice($step['next_after_drop'], 0, 5, true) as $idAction => $count) {
                $label = $idAction === 0 ? null : $reader->nameOf((int) $idAction);
                $next[] = ['exit' => $idAction === 0, 'label' => $label === null ? '' : StepMatcher::pathOf($label), 'visits' => $count];
            }
            $previous = $i > 0 ? $result['steps'][$i - 1]['reached'] : $step['reached'];
            $out[] = [
                'name' => $stepList[$i]['name'],
                'definition' => $stepList[$i],
                'visits' => $step['reached'],
                'rate_from_start' => $entered ? round($step['reached'] / $entered, 4) : 0,
                'rate_from_previous' => $previous ? round($step['reached'] / $previous, 4) : 0,
                'dropped' => $step['dropped'],
                'median_seconds_from_previous' => $step['median_seconds'],
                'next_after_drop' => $next,
                'dropped_visits' => $step['dropped_visits'],
            ];
        }
        $last = end($result['steps']);
        return [
            'idfunnel' => (int) $idFunnel,
            'name' => $name,
            'visits_in_period' => $result['visits'],
            'entered' => $entered,
            'converted' => $last['reached'],
            'conversion_rate' => $entered ? round($last['reached'] / $entered, 4) : 0,
            'steps' => $out,
        ];
    }

    /**
     * Parcours de pages les plus fréquents, niveau par niveau.
     *
     * @param int $depth nombre de niveaux (2 à 8)
     * @param int $maxNodes pages gardées par niveau, les autres sont regroupées
     * @param string $start chemin de la page de départ (« /produits ») ; vide = entrée sur le site
     * @param int $ignoreQuery 1 pour confondre les URL qui ne diffèrent que par leurs paramètres
     */
    public function getPaths($idSite, $period, $date, $segment = false, $depth = 5, $maxNodes = 6, $start = '', $ignoreQuery = 1)
    {
        Piwik::checkUserHasViewAccess($idSite);
        $reader = $this->reader((int) $idSite, $period, $date, $segment);
        $ignoreQuery = (bool) (int) $ignoreQuery;
        $start = trim(Common::unsanitizeInputValue((string) $start));
        $calculator = new PathCalculator((int) $depth, (int) $maxNodes, $start === '' ? null : self::pageLabel($start, $ignoreQuery));
        try {
            $reader->eachAction(function (array $row) use ($calculator, $reader, $ignoreQuery) {
                if (!empty($row['idaction_event_category']) || empty($row['idaction_url'])) {
                    return; // seules les pages vues dessinent le parcours
                }
                $name = $reader->nameOf((int) $row['idaction_url']);
                if ($name !== null) {
                    $calculator->push((int) $row['idvisit'], self::pageLabel(StepMatcher::pathOf($name), $ignoreQuery));
                }
            });
        } catch (\RuntimeException $e) {
            throw new \Exception(Piwik::translate($e->getMessage()));
        }
        return $calculator->result();
    }

    /** Pages, titres et événements fréquents de la période, pour l'aide à la saisie des étapes. */
    public function getSuggestions($idSite, $period, $date)
    {
        Piwik::checkUserHasViewAccess($idSite);
        $s = $this->reader((int) $idSite, $period, $date, '')->suggestions();
        $s['urls'] = array_values(array_unique(array_map([StepMatcher::class, 'pathOf'], $s['urls'])));
        return $s;
    }

    private static function pageLabel(string $path, bool $ignoreQuery): string
    {
        if ($ignoreQuery) {
            $path = preg_replace('/[?#].*$/', '', $path);
        }
        $path = $path === '' ? '/' : $path;
        return strlen($path) > 1 ? rtrim($path, '/') : $path;
    }

    private function reader(int $idSite, $period, $date, $segment): LogReader
    {
        $segment = $segment ? Common::unsanitizeInputValue((string) $segment) : '';
        return new LogReader($idSite, (string) $period, (string) $date, $segment);
    }

    private function decodeSteps($steps): array
    {
        if (is_string($steps)) {
            $steps = json_decode(Common::unsanitizeInputValue($steps), true);
        }
        try {
            return StepMatcher::sanitizeSteps($steps);
        } catch (\InvalidArgumentException $e) {
            throw new \Exception($e->getMessage());
        }
    }
}
