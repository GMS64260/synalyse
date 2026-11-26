# Synalyse

Mesure d'audience et analyse produit auto-hébergées : entonnoirs, parcours, rétention, enregistrements de session et heatmaps. Un binaire Rust et un PostgreSQL, là où les plateformes équivalentes demandent une quarantaine de conteneurs.

> Projet jeune : la mesure d'audience, les entonnoirs et les parcours fonctionnent, le reste de la feuille de route est à venir. Pas encore de version publiée.

## Pourquoi

PostHog répond au besoin, mais son auto-hébergement demande 16 Go de RAM, ClickHouse, Kafka et Redis. Matomo est léger, mais ses fonctions d'analyse produit sont payantes et propriétaires. Synalyse vise l'essentiel du premier avec l'empreinte du second.

## Principes

- **Léger d'abord** : tourne sur un petit serveur avec moins de 300 Mo de RAM, base comprise.
- **Fait pour grandir** : serveur sans état, événements partitionnés, stockage isolé derrière une interface pour accueillir un moteur en colonnes le jour où le volume l'exige.
- **Sans bandeau par défaut** : la mesure d'audience de base ne pose aucun cookie et ne reconnaît pas un visiteur d'un jour à l'autre. Les fonctions qui demandent un consentement (enregistrements, heatmaps, suivi dans le temps) ne s'activent qu'après acceptation.
- **Vos données chez vous** : aucune dépendance à un service tiers, l'adresse IP n'est jamais stockée.
- **Code original** : écrit de zéro, sans reprise de Matomo, de PostHog ni de leurs extensions.

## Feuille de route

| Lot | Contenu | État |
|---|---|---|
| 0 | Socle : schéma, organisations et projets, comptes, ingestion, traceur | Fait |
| 1 | Mesure d'audience et interface web | Fait |
| 1 bis | Compatibilité avec le traceur Matomo, import de l'historique, objectifs | À venir |
| 2 | Entonnoirs et parcours des visiteurs | Fait |
| 3 | Utilisateurs identifiés, rétention, cohortes | À venir |
| 4 | Enregistrements de session | À venir |
| 5 | Heatmaps | À venir |
| 6 | Capture automatique des clics, erreurs JavaScript | À venir |
| 7 | Feature flags, tests A/B, sondages | À venir |
| 8 | Entreprise : SSO, journal d'audit, rôles fins, stockage en colonnes | À venir |

## Essayer

Il faut Docker et Node.js 22 ou plus récent. La compilation se fait dans un conteneur.

```bash
bin/dev build                                   # compile le serveur
bin/dev run                                     # démarre Synalyse et son PostgreSQL sur http://127.0.0.1:8292
bin/dev cli admin create --email vous@exemple.fr   # crée votre compte, affiche son mot de passe
bin/dev test                                    # tests d'intégration
```

Sur vos pages, une seule ligne :

```html
<script async src="https://votre-synalyse.exemple/c.js" data-key="CLÉ_DU_SITE"></script>
```

Configuration par variables d'environnement : `SYNALYSE_DATABASE_URL` (obligatoire), `SYNALYSE_LISTEN`, `SYNALYSE_BASE_URL`, `SYNALYSE_TRUSTED_PROXIES`.

## Licence

AGPL v3 ou ultérieure. Voir [LICENSE](LICENSE). Si vous hébergez une version modifiée comme service, vous devez en publier les modifications.

Synalyse est un produit de [Synalto](https://synalto.eu), sans lien avec Matomo, InnoCraft ni PostHog.
