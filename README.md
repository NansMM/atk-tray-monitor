# atk-tray-monitor

Application Windows legere pour suivre la batterie d'une souris ATK F1 / LEVIATAN avec une interface minimale et une integration tray prioritaire.

## Stack

- Angular 21
- Tailwind CSS 4
- Tauri 2
- Rust
- `libatk-rs` pour la communication HID ATK

## Interface actuelle

- Titre dynamique base sur la souris detectee, avec un nom produit nettoye comme `ATK F1 LEVIATAN`
- Tag de status compact sur une seule ligne: `Charge`, `Batterie`, `Hors ligne`, `Connexion`, `Preview`
- Niveau de batterie avec jauge circulaire
- Petit historique batterie persistant entre les lancements
- Fenetre compacte pensee pour etre ouverte depuis l'icone tray, puis masquee automatiquement

## Comportement desktop

- Application tray-first sous Windows
- Fenetre principale masquee au lieu d'etre fermee
- Instance unique
- Lancement automatique optionnel avec Windows
- Reglages principaux dans le menu tray
- Notifications de batterie faible configurables

## Lecture batterie

- Detection heuristique des peripheriques ATK/VXE/F1 via HID
- Lecture batterie via `libatk-rs`
- Rafraichissement automatique toutes les 20 secondes cote frontend et backend
- Normalisation defensive des sauts aberrants observes au branchement pendant la charge

## Prerequis

1. Node.js LTS recommande
2. Rust via `rustup`
3. Visual Studio Build Tools avec le workload C++ Desktop

## Developpement

Frontend seul:

```bash
npm start
```

Application Tauri:

```bash
npm run tauri:dev
```

Build frontend:

```bash
npm run build
```

Build desktop:

```bash
npm run tauri:build
```

## GitHub Actions

- `CI` se lance sur chaque `push` vers `main` et sur chaque `pull_request`.
- Ce workflow installe Node.js 22 et Rust stable, puis execute `npm run build` et `cargo check --manifest-path src-tauri/Cargo.toml`.
- `Release Desktop` se lance manuellement ou sur un tag `v*` comme `v0.1.0`.
- Ce workflow produit les bundles Windows Tauri (`.msi` et installateur NSIS `.exe`), les publie en artefacts, et les attache automatiquement a la release GitHub sur les tags.

## Notes

- Un mode preview navigateur reste disponible pour travailler l'UI sans runtime Tauri.
- Si un nom HID brut de dongle remonte, le backend le remappe vers un nom produit plus lisible pour l'interface.

## Licence

Le backend utilise `libatk-rs`, qui est sous GPL-3.0. Si tu veux distribuer l'application en proprietaire, il faudra accepter cette contrainte ou remplacer cette dependance par une implementation maison ou une alternative plus permissive.
