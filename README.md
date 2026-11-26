# Synatomo

L'analyse produit qui manque à Matomo libre : entonnoirs, parcours, rétention, enregistrements de session et heatmaps, sous forme de plugins légers. Deux conteneurs, MariaDB pour seule base, moins de 1 Go de RAM.

Synatomo ne remplace pas Matomo, il s'y ajoute. Les plugins s'installent sur une instance Matomo 5 existante et calculent leurs rapports sur les visites déjà enregistrées : l'historique est conservé, rien n'est à migrer.

## Pourquoi

PostHog répond au besoin, mais son auto-hébergement demande 16 Go de RAM et une quarantaine de conteneurs (ClickHouse, Kafka, Postgres, Redis…). Les extensions équivalentes de Matomo sont payantes et propriétaires. Synatomo réécrit ces fonctions à partir de zéro, pour un serveur modeste.

## Principes

- **Léger** : pas de ClickHouse, pas de Kafka, pas de service en plus. Les rapports sont calculés en SQL sur les journaux de Matomo.
- **Sans bandeau par défaut** : la mesure d'audience de base reste sans cookie, dans le cadre de l'exemption de consentement de la CNIL. Les fonctions qui en sortent (enregistrements, heatmaps, suivi par visiteur) ne s'activent qu'après acceptation du visiteur.
- **Des plugins, pas un fork** : le cœur de Matomo n'est pas modifié, les mises à jour de sécurité s'appliquent sans refusion.
- **Code original** : aucune ligne ne vient des extensions payantes d'InnoCraft.

## Feuille de route

| Lot | Contenu | État |
|---|---|---|
| 0 | Socle : dépôt, instance de développement, jeu d'essai, tests | En cours |
| 1 | Entonnoirs et parcours des visiteurs | À venir |
| 2 | Rétention et cohortes | À venir |
| 3 | Enregistrements de session (`rrweb`), traceur commun, consentement | À venir |
| 4 | Heatmaps | À venir |
| 5 | Capture automatique des clics, erreurs JavaScript | À venir |
| 6 | Tests A/B, feature flags, sondages | Si le besoin se confirme |

## Développement

L'instance de développement est indépendante de toute production. Elle écoute sur `127.0.0.1:8291`.

```bash
bin/dev up        # démarre Matomo 5.14 et MariaDB
bin/dev install   # installe l'instance et relie les plugins du dépôt
bin/dev seed      # charge le jeu d'essai déterministe
bin/dev test      # lance les tests
bin/dev reset     # repart d'une base vide
```

Prérequis : Docker et Node.js 22 ou plus récent. Les secrets de l'instance vivent dans `dev/.env`, jamais versionné.

## Licence

GPL v3 ou ultérieure, comme Matomo. Voir [LICENSE](LICENSE).

Matomo est une marque de InnoCraft Ltd. Synatomo est un projet indépendant, sans lien avec InnoCraft ni avec le projet Matomo.
