<?php

/**
 * Synatomo — analyse produit pour Matomo.
 *
 * @license https://www.gnu.org/licenses/gpl-3.0.html GPL v3 or later
 */

namespace Piwik\Plugins\SynInsights\Engine;

/**
 * Agrège les parcours de pages des visites en un graphe par niveau (diagramme de flux).
 *
 * Chaque visite fournit la suite de ses pages (les rechargements consécutifs de la même
 * page comptent pour une seule). On garde les `depth` premières pages, à partir du début
 * de la visite ou de la première occurrence de la page de départ demandée.
 */
class PathCalculator
{
    public const OTHERS = '__autres__';
    public const EXIT = '__sortie__';

    private $depth;
    private $maxNodes;
    private $startLabel;
    private $visits = 0;
    private $matched = 0;
    /** @var array<int,array<string,int>> niveau => libellé => visites */
    private $nodes = [];
    /** @var array<int,array<string,array<string,int>>> niveau => de => vers => visites */
    private $links = [];

    private $currentVisit = null;
    private $sequence = [];

    /**
     * @param int $depth nombre de niveaux affichés (2 à 8)
     * @param int $maxNodes pages distinctes gardées par niveau, le reste est regroupé
     * @param string|null $startLabel libellé de la page de départ, ou null pour partir de l'entrée sur le site
     */
    public function __construct(int $depth = 5, int $maxNodes = 6, ?string $startLabel = null)
    {
        $this->depth = max(2, min(8, $depth));
        $this->maxNodes = max(2, min(15, $maxNodes));
        $this->startLabel = $startLabel === null || $startLabel === '' ? null : $startLabel;
    }

    /** @param string $label libellé de la page déjà normalisé par l'appelant */
    public function push(int $idVisit, string $label): void
    {
        if ($idVisit !== $this->currentVisit) {
            $this->closeVisit();
            $this->currentVisit = $idVisit;
            $this->sequence = [];
        }
        if ($this->sequence && end($this->sequence) === $label) {
            return;
        }
        $this->sequence[] = $label;
    }

    private function closeVisit(): void
    {
        if ($this->currentVisit === null) {
            return;
        }
        $this->visits++;
        $seq = $this->sequence;
        if ($this->startLabel !== null) {
            $pos = array_search($this->startLabel, $seq, true);
            if ($pos === false) {
                return;
            }
            $seq = array_slice($seq, $pos);
        }
        if (!$seq) {
            return;
        }
        $this->matched++;
        $ended = count($seq) <= $this->depth;
        $seq = array_slice($seq, 0, $this->depth);
        foreach ($seq as $level => $label) {
            $this->nodes[$level][$label] = ($this->nodes[$level][$label] ?? 0) + 1;
            if ($level + 1 < count($seq)) {
                $to = $seq[$level + 1];
            } elseif ($ended && $level + 1 < $this->depth) {
                $to = self::EXIT;
            } else {
                continue;
            }
            $this->links[$level][$label][$to] = ($this->links[$level][$label][$to] ?? 0) + 1;
        }
    }

    /**
     * @return array{visits:int, matched:int, depth:int, levels:array<int,array<int,array{label:string,visits:int}>>, links:array<int,array{level:int,from:string,to:string,visits:int}>}
     */
    public function result(): array
    {
        $this->closeVisit();
        $this->currentVisit = null;

        // Par niveau : garder les pages les plus vues, regrouper les autres.
        $kept = [];
        $levels = [];
        for ($level = 0; $level < $this->depth; $level++) {
            $counts = $this->nodes[$level] ?? [];
            if (!$counts) {
                break;
            }
            // Tri stable et déterministe : visites décroissantes puis libellé.
            uksort($counts, function ($a, $b) use ($counts) {
                return $counts[$b] <=> $counts[$a] ?: strcmp((string) $a, (string) $b);
            });
            $top = array_slice($counts, 0, $this->maxNodes, true);
            $rest = array_sum($counts) - array_sum($top);
            $kept[$level] = $top;
            $levels[$level] = [];
            foreach ($top as $label => $n) {
                $levels[$level][] = ['label' => (string) $label, 'visits' => $n];
            }
            if ($rest > 0) {
                $levels[$level][] = ['label' => self::OTHERS, 'visits' => $rest];
            }
        }

        $merged = [];
        foreach ($this->links as $level => $fromList) {
            if (!isset($kept[$level])) {
                continue;
            }
            foreach ($fromList as $from => $toList) {
                $f = isset($kept[$level][$from]) ? (string) $from : self::OTHERS;
                foreach ($toList as $to => $n) {
                    $t = ($to === self::EXIT || isset($kept[$level + 1][$to])) ? (string) $to : self::OTHERS;
                    $key = $level . "\0" . $f . "\0" . $t;
                    $merged[$key] = ($merged[$key] ?? 0) + $n;
                }
            }
        }
        $links = [];
        foreach ($merged as $key => $n) {
            [$level, $f, $t] = explode("\0", $key);
            $links[] = ['level' => (int) $level, 'from' => $f, 'to' => $t, 'visits' => $n];
        }
        usort($links, function ($a, $b) {
            return [$a['level'], $b['visits'], $a['from'], $a['to']] <=> [$b['level'], $a['visits'], $b['from'], $b['to']];
        });

        return ['visits' => $this->visits, 'matched' => $this->matched, 'depth' => count($levels), 'levels' => array_values($levels), 'links' => $links];
    }
}
