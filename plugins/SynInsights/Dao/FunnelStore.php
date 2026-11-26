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

/** Entonnoirs enregistrés (table syn_funnel). Toujours adressés par identifiant et par site. */
class FunnelStore
{
    private function table(): string
    {
        return Common::prefixTable('syn_funnel');
    }

    public function all(int $idSite): array
    {
        $rows = Db::fetchAll(
            'SELECT idfunnel, idsite, name, steps, created_at, updated_at FROM ' . $this->table() . ' WHERE idsite = ? AND deleted = 0 ORDER BY name, idfunnel',
            [$idSite]
        );
        return array_map([$this, 'hydrate'], $rows);
    }

    public function get(int $idSite, int $idFunnel): ?array
    {
        $row = Db::fetchRow(
            'SELECT idfunnel, idsite, name, steps, created_at, updated_at FROM ' . $this->table() . ' WHERE idsite = ? AND idfunnel = ? AND deleted = 0',
            [$idSite, $idFunnel]
        );
        return $row ? $this->hydrate($row) : null;
    }

    public function create(int $idSite, string $name, array $steps): int
    {
        $now = Date::now()->getDatetime();
        Db::query(
            'INSERT INTO ' . $this->table() . ' (idsite, name, steps, created_at, updated_at) VALUES (?, ?, ?, ?, ?)',
            [$idSite, $name, json_encode($steps, JSON_UNESCAPED_UNICODE), $now, $now]
        );
        return (int) Db::get()->lastInsertId();
    }

    public function update(int $idSite, int $idFunnel, string $name, array $steps): void
    {
        Db::query(
            'UPDATE ' . $this->table() . ' SET name = ?, steps = ?, updated_at = ? WHERE idsite = ? AND idfunnel = ? AND deleted = 0',
            [$name, json_encode($steps, JSON_UNESCAPED_UNICODE), Date::now()->getDatetime(), $idSite, $idFunnel]
        );
    }

    public function delete(int $idSite, int $idFunnel): void
    {
        Db::query('UPDATE ' . $this->table() . ' SET deleted = 1, updated_at = ? WHERE idsite = ? AND idfunnel = ?', [Date::now()->getDatetime(), $idSite, $idFunnel]);
    }

    private function hydrate(array $row): array
    {
        return [
            'idfunnel' => (int) $row['idfunnel'],
            'idsite' => (int) $row['idsite'],
            'name' => $row['name'],
            'steps' => json_decode($row['steps'], true) ?: [],
            'created_at' => $row['created_at'],
            'updated_at' => $row['updated_at'],
        ];
    }
}
