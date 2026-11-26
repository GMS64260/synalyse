# Synalyse

Mesure d'audience et analyse produit auto-hébergées : entonnoirs, parcours, rétention, enregistrements de session et heatmaps. Un binaire Rust et un PostgreSQL, là où les plateformes équivalentes demandent une quarantaine de conteneurs.

> Le projet démarre. Rien n'est encore utilisable ; ce fichier décrit la cible.

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
| 0 | Socle : schéma, organisations et projets, comptes, ingestion, traceur | Cadrage |
| 1 | Mesure d'audience, compatibilité avec le traceur Matomo, import de l'historique | À venir |
| 2 | Entonnoirs et parcours des visiteurs | À venir |
| 3 | Utilisateurs identifiés, rétention, cohortes | À venir |
| 4 | Enregistrements de session | À venir |
| 5 | Heatmaps | À venir |
| 6 | Capture automatique des clics, erreurs JavaScript | À venir |
| 7 | Feature flags, tests A/B, sondages | À venir |
| 8 | Entreprise : SSO, journal d'audit, rôles fins, stockage en colonnes | À venir |

## Licence

AGPL v3 ou ultérieure. Voir [LICENSE](LICENSE). Si vous hébergez une version modifiée comme service, vous devez en publier les modifications.

Synalyse est un produit de [Synalto](https://synalto.eu), sans lien avec Matomo, InnoCraft ni PostHog.
