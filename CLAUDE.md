# Synalyse

Mesure d'audience et analyse produit auto-hébergées (Rust, PostgreSQL). Produit de Synalto, dépôt public, AGPL v3.
Contexte, décisions, lots et pièges : vault, `/var/www/vault/Projets/synalyse/` (`synalyse.md`, `Synalyse-cadrage.md`).

## Avant d'agir

- **Ne jamais compiler sur l'hôte** : passer par `bin/dev` (conteneur plafonné à 2,5 Go). Un build libre peut geler le VPS. Un seul build à la fois sur la machine : prévenir les autres sessions.
- **Rien n'est en production.** La production de mesure d'audience reste Matomo (`/var/www/matomo`) : ne pas y toucher depuis ce projet.
- **Dépôt public** : aucun secret, aucune donnée de client, aucune adresse interne dans le code, les tests ou les messages de commit.
- **Code original uniquement** : ne pas lire ni copier le code des extensions payantes de Matomo ni les dossiers `ee/` de PostHog.
- Vie privée : l'adresse IP d'un visiteur ne doit jamais être stockée ni journalisée ; sans consentement, rien n'est écrit dans son navigateur.

## Commandes

    bin/dev build            # compile (debug)
    bin/dev unit             # tests unitaires Rust
    bin/dev run              # serveur de dev sur 127.0.0.1:8292 et son PostgreSQL
    bin/dev test             # tests d'intégration (node --test) contre le serveur de dev
    bin/dev cargo fmt        # format ; bin/dev cargo clippy --all-targets -- -D warnings
    bin/dev stop             # arrête les conteneurs de dev

Pousser : `git -c credential.helper='!gh auth git-credential' push origin main`.
