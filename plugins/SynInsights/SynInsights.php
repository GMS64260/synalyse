<?php

/**
 * Synatomo — analyse produit pour Matomo.
 *
 * @license https://www.gnu.org/licenses/gpl-3.0.html GPL v3 or later
 */

namespace Piwik\Plugins\SynInsights;

use Piwik\Common;
use Piwik\Db;
use Piwik\DbHelper;
use Piwik\Plugin;

class SynInsights extends Plugin
{
    public function registerEvents()
    {
        return [
            'AssetManager.getStylesheetFiles' => 'getStylesheetFiles',
            'AssetManager.getJavaScriptFiles' => 'getJavaScriptFiles',
            'Translate.getClientSideTranslationKeys' => 'getClientSideTranslationKeys',
        ];
    }

    public function install()
    {
        DbHelper::createTable('syn_funnel', "
            `idfunnel` INT UNSIGNED NOT NULL AUTO_INCREMENT,
            `idsite` INT UNSIGNED NOT NULL,
            `name` VARCHAR(100) NOT NULL,
            `steps` TEXT NOT NULL,
            `created_at` DATETIME NOT NULL,
            `updated_at` DATETIME NOT NULL,
            `deleted` TINYINT UNSIGNED NOT NULL DEFAULT 0,
            PRIMARY KEY (`idfunnel`),
            KEY `idsite` (`idsite`, `deleted`)
        ");
    }

    public function uninstall()
    {
        Db::dropTables([Common::prefixTable('syn_funnel')]);
    }

    public function getStylesheetFiles(&$stylesheets)
    {
        $stylesheets[] = 'plugins/SynInsights/stylesheets/syninsights.less';
    }

    public function getJavaScriptFiles(&$jsFiles)
    {
        $jsFiles[] = 'plugins/SynInsights/javascripts/syninsights.js';
    }

    public function getClientSideTranslationKeys(&$keys)
    {
        $file = __DIR__ . '/lang/en.json';
        $strings = json_decode((string) file_get_contents($file), true);
        foreach (array_keys($strings['SynInsights'] ?? []) as $key) {
            $keys[] = 'SynInsights_' . $key;
        }
    }
}
