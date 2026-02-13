# Huawei Importer

Outil en ligne de commande écrit en Rust permettant d'importer les données de production et consommation d'énergie solaire, exportées depuis la plateforme Huawei FusionSolar, dans une base de données PostgreSQL avec [TimescaleDB](https://www.timescale.com/).

## Fonctionnement

Le programme :

1. **Parcourt** un répertoire à la recherche de fichiers JSON nommés au format `YYYY.MM.json` (ex : `2025.02.json`).
2. **Extrait** les données journalières de chaque fichier et calcule trois mesures :
   - `accumulated_solar_energy` — énergie solaire produite (`productPower`)
   - `active_energy_exported` — énergie exportée vers le réseau (`productPower - selfUsePower`)
   - `active_energy_imported` — énergie importée depuis le réseau (`usePower - selfUsePower`)
3. **Insère** les lignes dans la table TimescaleDB `_timescaledb_internal._materialized_hypertable_3` avec un `ON CONFLICT DO NOTHING` pour éviter les doublons.

Chaque jour du mois génère jusqu'à 3 lignes (une par mesure), à condition que les valeurs ne soient pas `"--"` (donnée absente).

## Utilisation

### Prérequis

- Rust (édition 2021+)
- PostgreSQL avec l'extension TimescaleDB
- [just](https://github.com/casey/just) (optionnel, pour les commandes raccourcies)

### Configuration

Copiez `.env.local.exemple` en `.env.local` et renseignez vos paramètres :

```bash
export DATABASE_URL=postgresql://user:pass@host:5432/dbname
export DATA_DIR=./data/
```

### Commandes

```bash
# Compilation
cargo build --release

# Mode dry-run (affichage sans insertion)
cargo run -- --data-dir ./data/ --dry-run

# Import réel en base
cargo run -- --data-dir ./data/ --db-url "postgresql://user:pass@host:5432/dbname"
```

Avec `just` (les variables d'environnement sont lues depuis `.env.local`) :

```bash
just dry-run    # Dry-run
just run        # Import réel
just lint       # Format + Clippy
just ci         # Lint + Tests + Build
```

### Options CLI

| Option       | Description                                          | Requis                  |
|------------- |------------------------------------------------------|-------------------------|
| `--data-dir` | Répertoire contenant les fichiers JSON (défaut : `.`) | Non                     |
| `--db-url`   | URL de connexion PostgreSQL                          | Oui (sauf en dry-run)   |
| `--dry-run`  | Affiche les données sans les insérer                 | Non                     |

## Format des fichiers JSON d'importation

Chaque fichier doit être nommé **`YYYY.MM.json`** (ex : `2024.03.json` pour mars 2024).

### Structure minimale requise

```json
{
  "data": {
    "productPower": ["25.74", "29.23", "30.22", "..."],
    "usePower": ["29.66", "30.02", "31.98", "..."],
    "selfUsePower": ["24.57", "25.95", "27.42", "..."]
  }
}
```

### Détail des champs obligatoires

| Champ            | Type       | Description                                                         |
|------------------|------------|---------------------------------------------------------------------|
| `productPower`   | `string[]` | Énergie solaire produite (kWh), un élément par jour du mois         |
| `usePower`       | `string[]` | Énergie totale consommée (kWh), un élément par jour du mois         |
| `selfUsePower`   | `string[]` | Énergie solaire auto-consommée (kWh), un élément par jour du mois   |

### Règles de format

- Les **valeurs** sont des **strings** représentant des nombres décimaux (ex : `"25.74"`).
- La valeur `"--"` indique une donnée absente ; la ligne correspondante sera ignorée.
- Chaque tableau doit contenir **autant d'éléments que de jours dans le mois** (28 à 31). Les jours excédentaires (ex : 31 février) sont automatiquement ignorés.
- Le fichier JSON complet tel qu'exporté par FusionSolar contient de nombreux autres champs (`success`, `stationDn`, `xAxis`, etc.) qui sont simplement ignorés par l'importateur.

### Exemple complet (février 2025)

```json
{
  "success": true,
  "data": {
    "productPower": ["25.74", "29.23", "30.22", "30.64", "30.92", "31.20", "24.68", "17.24", "31.39", "15.07", "16.04", "14.88", "12.02", "29.66", "37.02", "32.01", "36.39", "37.42", "35.59", "27.39", "33.26", "27.89", "18.82", "22.13", "12.70", "29.17", "29.89", "43.23"],
    "usePower": ["29.66", "30.02", "31.98", "23.41", "27.81", "31.76", "26.88", "14.57", "23.59", "20.16", "15.91", "16.23", "16.17", "33.85", "38.21", "30.40", "34.21", "22.82", "25.68", "16.24", "30.42", "14.11", "18.74", "22.75", "15.54", "22.25", "21.56", "23.85"],
    "selfUsePower": ["24.57", "25.95", "27.42", "18.69", "23.44", "27.75", "21.34", "8.92", "18.94", "14.28", "10.37", "11.11", "10.41", "27.95", "31.60", "25.26", "29.43", "17.56", "21.74", "11.82", "25.59", "8.09", "13.74", "17.20", "9.71", "17.59", "16.52", "19.73"]
  }
}
```

## Schéma de la table cible

La table `_timescaledb_internal._materialized_hypertable_3` attend les colonnes suivantes :

| Colonne       | Type            | Description                                                    |
|---------------|-----------------|----------------------------------------------------------------|
| `bucket`      | `timestamptz`   | Date du jour (à 00:00 UTC)                                    |
| `source`      | `text`          | Source de la mesure (`solar_meter` ou `energy_meter`)           |
| `measurement` | `text`          | Type de mesure (voir ci-dessus)                                |
| `value`       | `double precision` | Valeur en kWh                                               |

## Licence

Ce projet est sous licence MIT. Voir le fichier LICENSE pour plus de détails.
