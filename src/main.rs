use anyhow::{Context, Result};
use chrono::{NaiveDate, TimeZone, Utc};
use clap::Parser;
use glob::glob;
use serde::Deserialize;
use std::path::PathBuf;
use tokio_postgres::NoTls;

/// Importer les données JSON Huawei dans une table TimescaleDB
#[derive(Parser, Debug)]
#[command(name = "huawei-importer", version, about)]
struct Args {
    /// URL de connexion PostgreSQL (ex: postgresql://user:pass@host/db)
    #[arg(long)]
    db_url: Option<String>,

    /// Répertoire contenant les fichiers JSON
    #[arg(long, default_value = ".")]
    data_dir: PathBuf,

    /// Mode dry-run : affiche les données sans les insérer
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
struct HuaweiFile {
    data: HuaweiData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HuaweiData {
    product_power: Vec<StringOrDash>,
    use_power: Vec<StringOrDash>,
    self_use_power: Vec<StringOrDash>,
}

/// Représente une valeur qui peut être un nombre sous forme de string ou "--"
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StringOrDash {
    Value(String),
}

impl StringOrDash {
    fn as_f64(&self) -> Option<f64> {
        match self {
            StringOrDash::Value(s) if s == "--" => None,
            StringOrDash::Value(s) => s.parse::<f64>().ok(),
        }
    }
}

/// Représente une ligne à insérer dans la table
#[derive(Debug)]
struct Row {
    bucket: NaiveDate,
    source: &'static str,
    measurement: &'static str,
    value: f64,
}

/// Parse le nom de fichier YYYY.MM.json et retourne (année, mois)
fn parse_filename(path: &std::path::Path) -> Option<(i32, u32)> {
    let stem = path.file_stem()?.to_str()?;
    let parts: Vec<&str> = stem.split('.').collect();
    if parts.len() != 2 {
        return None;
    }
    let year = parts[0].parse::<i32>().ok()?;
    let month = parts[1].parse::<u32>().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some((year, month))
}

/// Extrait les lignes à insérer depuis un fichier JSON
fn extract_rows(path: &std::path::Path) -> Result<Vec<Row>> {
    let (year, month) =
        parse_filename(path).with_context(|| format!("Nom de fichier invalide : {:?}", path))?;

    let content =
        std::fs::read_to_string(path).with_context(|| format!("Impossible de lire {:?}", path))?;

    let file: HuaweiFile =
        serde_json::from_str(&content).with_context(|| format!("JSON invalide dans {:?}", path))?;

    let data = &file.data;
    let len = data
        .product_power
        .len()
        .min(data.use_power.len())
        .min(data.self_use_power.len());

    let mut rows = Vec::new();

    for i in 0..len {
        let day = (i + 1) as u32;
        let date = match NaiveDate::from_ymd_opt(year, month, day) {
            Some(d) => d,
            None => continue, // jour invalide (ex: 31 février)
        };

        let prod = data.product_power[i].as_f64();
        let usage = data.use_power[i].as_f64();
        let self_use = data.self_use_power[i].as_f64();

        // accumulated_solar_energy = productPower
        if let Some(prod_val) = prod {
            rows.push(Row {
                bucket: date,
                source: "solar_meter",
                measurement: "accumulated_solar_energy",
                value: prod_val,
            });
        }

        // active_energy_exported = productPower - selfUsePower
        if let (Some(prod_val), Some(self_use_val)) = (prod, self_use) {
            rows.push(Row {
                bucket: date,
                source: "energy_meter",
                measurement: "active_energy_exported",
                value: prod_val - self_use_val,
            });
        }

        // active_energy_imported = usePower - selfUsePower
        if let (Some(use_val), Some(self_use_val)) = (usage, self_use) {
            rows.push(Row {
                bucket: date,
                source: "energy_meter",
                measurement: "active_energy_imported",
                value: use_val - self_use_val,
            });
        }
    }

    Ok(rows)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Chercher tous les fichiers YYYY.MM.json
    let pattern = args.data_dir.join("*.json");
    let pattern_str = pattern
        .to_str()
        .context("Chemin invalide pour le pattern glob")?;

    let mut json_files: Vec<PathBuf> = glob(pattern_str)
        .context("Pattern glob invalide")?
        .filter_map(|entry| entry.ok())
        .filter(|path| parse_filename(path).is_some())
        .collect();

    json_files.sort();

    if json_files.is_empty() {
        eprintln!("Aucun fichier YYYY.MM.json trouvé dans {:?}", args.data_dir);
        return Ok(());
    }

    eprintln!("Trouvé {} fichier(s) JSON à traiter", json_files.len());

    // Extraire toutes les lignes
    let mut all_rows: Vec<Row> = Vec::new();
    for path in &json_files {
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        match extract_rows(path) {
            Ok(rows) => {
                eprintln!("  {} : {} lignes extraites", file_name, rows.len());
                all_rows.extend(rows);
            }
            Err(e) => {
                eprintln!("  {} : ERREUR - {}", file_name, e);
            }
        }
    }

    eprintln!("Total : {} lignes à insérer", all_rows.len());

    if args.dry_run {
        eprintln!("\n=== MODE DRY-RUN ===\n");
        println!(
            "{:<12} {:<15} {:<30} {:>10}",
            "bucket", "source", "measurement", "value"
        );
        println!("{}", "-".repeat(70));
        for row in &all_rows {
            println!(
                "{:<12} {:<15} {:<30} {:>10.2}",
                row.bucket, row.source, row.measurement, row.value
            );
        }
        return Ok(());
    }

    // Connexion PostgreSQL
    let db_url = args
        .db_url
        .as_deref()
        .context("--db-url est requis (sauf en mode --dry-run)")?;

    let (mut client, connection) = tokio_postgres::connect(db_url, NoTls)
        .await
        .context("Impossible de se connecter à PostgreSQL")?;

    // Gérer la connexion en arrière-plan
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Erreur de connexion PostgreSQL : {}", e);
        }
    });

    // Insertion par batch dans une transaction
    let transaction = client
        .transaction()
        .await
        .context("Impossible de démarrer une transaction")?;

    let statement = transaction
        .prepare(
            "INSERT INTO _timescaledb_internal._materialized_hypertable_3 (bucket, source, measurement, value) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT DO NOTHING",
        )
        .await
        .context("Impossible de préparer la requête INSERT")?;

    let mut inserted = 0u64;
    for row in &all_rows {
        let naive_dt = row
            .bucket
            .and_hms_opt(0, 0, 0)
            .context("Impossible de créer le timestamp")?;
        let timestamp = Utc.from_utc_datetime(&naive_dt);

        transaction
            .execute(
                &statement,
                &[&timestamp, &row.source, &row.measurement, &row.value],
            )
            .await
            .with_context(|| {
                format!(
                    "Erreur lors de l'insertion de {} {} {} {}",
                    row.bucket, row.source, row.measurement, row.value
                )
            })?;
        inserted += 1;
    }

    transaction
        .commit()
        .await
        .context("Impossible de valider la transaction")?;

    eprintln!("{} lignes insérées avec succès !", inserted);

    Ok(())
}
