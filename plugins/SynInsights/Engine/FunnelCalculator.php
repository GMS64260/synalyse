<?php

/**
 * Synatomo — analyse produit pour Matomo.
 *
 * @license https://www.gnu.org/licenses/gpl-3.0.html GPL v3 or later
 */

namespace Piwik\Plugins\SynInsights\Engine;

/**
 * Calcule un entonnoir sur un flux d'actions trié par visite puis par ordre chronologique.
 *
 * Règle : une visite franchit l'étape N si, après l'action qui a validé l'étape N-1,
 * elle contient une action qui correspond à l'étape N. D'autres actions peuvent
 * s'intercaler. Chaque étape prend la première action possible, ce qui maximise
 * le nombre d'étapes franchies.
 */
class FunnelCalculator
{
    /** Nombre de visites gardées en exemple par étape (abandons et passages). */
    public const SAMPLE_SIZE = 50;

    /** @var StepMatcher */
    private $matcher;
    private $n;
    private $reached;
    private $durations;
    private $nextAfterDrop;
    private $droppedSample;
    private $visits = 0;

    // État de la visite en cours.
    private $currentVisit = null;
    private $stepIndex = 0;
    private $lastMatchTs = 0;
    private $awaitingNext = false;
    private $nextAction = null;

    public function __construct(StepMatcher $matcher)
    {
        $this->matcher = $matcher;
        $this->n = $matcher->count();
        $this->reached = array_fill(0, $this->n, 0);
        $this->durations = array_fill(0, $this->n, []);
        $this->nextAfterDrop = array_fill(0, $this->n, []);
        $this->droppedSample = array_fill(0, $this->n, []);
    }

    /** @param array $row idvisit, ts (Unix), idaction_url, idaction_name, idaction_event_category, idaction_event_action */
    public function push(array $row): void
    {
        $idVisit = (int) $row['idvisit'];
        if ($idVisit !== $this->currentVisit) {
            $this->closeVisit();
            $this->currentVisit = $idVisit;
            $this->visits++;
            $this->stepIndex = 0;
            $this->lastMatchTs = 0;
            $this->awaitingNext = false;
            $this->nextAction = null;
        }

        if ($this->stepIndex < $this->n && $this->matcher->matches($this->stepIndex, $row)) {
            $ts = (int) $row['ts'];
            $this->reached[$this->stepIndex]++;
            if ($this->stepIndex > 0) {
                $this->durations[$this->stepIndex][] = max(0, $ts - $this->lastMatchTs);
            }
            $this->lastMatchTs = $ts;
            $this->stepIndex++;
            $this->awaitingNext = true;
            $this->nextAction = null;
            return;
        }

        // Première action de page après la dernière étape franchie : sert à dire où partent ceux qui abandonnent.
        if ($this->awaitingNext && $this->stepIndex > 0 && empty($row['idaction_event_category']) && !empty($row['idaction_url'])) {
            $this->nextAction = (int) $row['idaction_url'];
            $this->awaitingNext = false;
        }
    }

    private function closeVisit(): void
    {
        if ($this->currentVisit === null || $this->stepIndex === 0 || $this->stepIndex >= $this->n) {
            return;
        }
        $last = $this->stepIndex - 1; // dernière étape franchie, la visite a abandonné ensuite
        $key = $this->nextAction === null ? 0 : $this->nextAction; // 0 = sortie du site
        $this->nextAfterDrop[$last][$key] = ($this->nextAfterDrop[$last][$key] ?? 0) + 1;
        if (count($this->droppedSample[$last]) < self::SAMPLE_SIZE) {
            $this->droppedSample[$last][] = $this->currentVisit;
        }
    }

    /**
     * @return array{visits:int, steps:array<int,array{reached:int, dropped:int, median_seconds:?int, next_after_drop:array<int,int>, dropped_visits:int[]}>}
     */
    public function result(): array
    {
        $this->closeVisit();
        $this->currentVisit = null;
        $steps = [];
        for ($i = 0; $i < $this->n; $i++) {
            $next = $i + 1 < $this->n ? $this->reached[$i + 1] : null;
            arsort($this->nextAfterDrop[$i]);
            $steps[] = [
                'reached' => $this->reached[$i],
                'dropped' => $next === null ? 0 : $this->reached[$i] - $next,
                'median_seconds' => self::median($this->durations[$i]),
                'next_after_drop' => $this->nextAfterDrop[$i],
                'dropped_visits' => $this->droppedSample[$i],
            ];
        }
        return ['visits' => $this->visits, 'steps' => $steps];
    }

    public static function median(array $values): ?int
    {
        $count = count($values);
        if ($count === 0) {
            return null;
        }
        sort($values);
        $mid = intdiv($count, 2);
        return $count % 2 ? (int) $values[$mid] : (int) round(($values[$mid - 1] + $values[$mid]) / 2);
    }
}
