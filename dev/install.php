<?php
// Installe l'instance de DÉVELOPPEMENT sans l'assistant web. Idempotent.
// Lancé par `bin/dev install` dans le conteneur, en www-data. Deux phases dans
// deux processus : Matomo ne charge sa liste complète de plugins qu'une fois
// config.ini.php présent.
use Piwik\Access;
use Piwik\Application\Environment;
use Piwik\Config;
use Piwik\DbHelper;
use Piwik\Plugin\Manager;
use Piwik\Plugins\SitesManager\API as SitesApi;
use Piwik\Plugins\UsersManager\API as UsersApi;
use Piwik\Updater;
use Piwik\Version;

define('PIWIK_DOCUMENT_ROOT', '/var/www/html');
define('PIWIK_INCLUDE_PATH', PIWIK_DOCUMENT_ROOT);
require_once PIWIK_INCLUDE_PATH . '/core/bootstrap.php';

$phase = $argv[1] ?? 'config';

if ($phase === 'config') {
    $env = new Environment('cli');
    $env->init();
    $config = Config::getInstance();
    // L'assistant web crée un config.ini.php sans base dès la première visite : on teste l'hôte.
    if (empty($config->database['host'])) {
        $config->database = [
            'host' => 'db', 'username' => 'matomo', 'password' => getenv('SYN_DB_PASSWORD'),
            'dbname' => 'matomo', 'tables_prefix' => 'matomo_', 'adapter' => 'PDO\MYSQL',
            'port' => 3306, 'charset' => 'utf8mb4', 'collation' => 'utf8mb4_general_ci',
        ];
        $config->General['installation_in_progress'] = 1;
        $config->General['salt'] = bin2hex(random_bytes(16));
        $config->General['trusted_hosts'] = ['127.0.0.1:8291', 'localhost:8291'];
        $config->forceSave();
        echo "config.ini.php écrit\n";
    }
    passthru('php ' . escapeshellarg(__FILE__) . ' tables', $code);
    exit($code);
}

$env = new Environment('cli');
$env->init();

if ($phase === 'tables') {
    if (!in_array('matomo_site', DbHelper::getTablesInstalled(), true)) {
        DbHelper::createTables();
        DbHelper::createAnonymousUser();
        DbHelper::recordInstallVersion();
        Updater::recordComponentSuccessfullyUpdated('core', Version::VERSION);
        echo "tables créées\n";
    }
    $config = Config::getInstance();
    if (!empty($config->General['installation_in_progress'])) {
        $general = $config->General;
        unset($general['installation_in_progress']);
        $config->General = $general;
        $config->forceSave();
    }
    passthru('php ' . escapeshellarg(__FILE__) . ' data', $code);
    exit($code);
}

if ($phase === 'data') {
    Manager::getInstance()->loadActivatedPlugins();
    Manager::getInstance()->installLoadedPlugins();
    Access::doAsSuperUser(function () {
        $users = UsersApi::getInstance();
        if (!$users->userExists('dev')) {
            $users->addUser('dev', getenv('SYN_ADMIN_PASSWORD'), 'dev@synatomo.test');
            $users->setSuperUserAccess('dev', true);
            echo "compte dev créé\n";
        }
        $sites = SitesApi::getInstance();
        if (count($sites->getAllSitesId()) === 0) {
            $id = $sites->addSite('Boutique démo', ['https://demo.test'], 0, null, null, null, null, null, 'Europe/Paris', 'EUR');
            // Le jeu d'essai est antérieur à l'installation : reculer la date de création du site.
            \Piwik\Db::query('UPDATE ' . \Piwik\Common::prefixTable('site') . " SET ts_created = '2026-10-01 00:00:00' WHERE idsite = ?", [$id]);
            echo "site $id créé\n";
        }
        // Réglages de confidentialité alignés sur la production (exemption CNIL).
        $privacy = new \Piwik\Plugins\PrivacyManager\Config();
        $privacy->ipAnonymizerEnabled = true;
        $privacy->ipAddressMaskLength = 2;
        $privacy->useAnonymizedIpForVisitEnrichment = true;
        $privacy->forceCookielessTracking = true;
        $privacy->anonymizeUserId = true;
        // Jeton d'API pour le jeu d'essai et les tests, écrit dans dev/.token par bin/dev.
        if (in_array('--token', $GLOBALS['argv'], true)) {
            $model = new \Piwik\Plugins\UsersManager\Model();
            $token = $model->generateRandomTokenAuth();
            $model->addTokenAuth('dev', $token, 'synatomo-dev', \Piwik\Date::now()->getDatetime());
            fwrite(STDERR, "TOKEN=$token\n");
        }
    });
    echo "installation terminée\n";
}
