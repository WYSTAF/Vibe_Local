//! Single unified SQLite schema for the whole Vibe Local ecosystem.
//! WAL mode on disk, `busy_timeout = 5000`, single-owner access pattern
//! (the connection is shared behind one `Arc<Mutex<..>>` by the server).

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

use crate::catalog::CatalogEntry;
use crate::hardware::HardwareReport;
use crate::util::iso8601_now;

pub type SharedDb = Arc<Mutex<Db>>;

pub struct Db {
    pub conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS hardware_scans (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL,
    os TEXT NOT NULL,
    cpu_model TEXT NOT NULL,
    ram_total_mb INTEGER NOT NULL,
    ram_free_mb INTEGER NOT NULL,
    preferred_backend TEXT NOT NULL,
    tier TEXT NOT NULL,
    report_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS models (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    hf_repo TEXT NOT NULL,
    filename TEXT NOT NULL,
    quant TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    architecture TEXT,
    family TEXT,
    total_params_b REAL,
    active_params_b REAL,
    context_length INTEGER,
    download_url TEXT NOT NULL,
    downloads INTEGER NOT NULL DEFAULT 0,
    likes INTEGER NOT NULL DEFAULT 0,
    trusted INTEGER NOT NULL DEFAULT 0,
    discovered_ts TEXT NOT NULL,
    UNIQUE(hf_repo, filename)
);
CREATE TABLE IF NOT EXISTS gguf_inspections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL,
    path TEXT NOT NULL,
    architecture TEXT,
    quant TEXT,
    info_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS benchmarks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL,
    model_id TEXT NOT NULL,
    hf_repo TEXT,
    filename TEXT,
    quant TEXT,
    hardware_profile TEXT,
    backend TEXT,
    pillar TEXT NOT NULL,
    score REAL NOT NULL,
    metrics_json TEXT NOT NULL,
    provenance TEXT NOT NULL DEFAULT 'measured',
    source_url TEXT
);
CREATE TABLE IF NOT EXISTS vibemark_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL,
    model_id TEXT NOT NULL,
    backend TEXT NOT NULL,
    engine TEXT NOT NULL,
    prefill_tps REAL NOT NULL,
    decode_tps REAL NOT NULL,
    ttft_ms REAL NOT NULL,
    peak_ram_mb REAL NOT NULL,
    duration_s REAL NOT NULL,
    provenance TEXT NOT NULL DEFAULT 'measured'
);
CREATE TABLE IF NOT EXISTS catalog_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    model_id TEXT NOT NULL,
    quant TEXT NOT NULL,
    hardware_profile TEXT NOT NULL,
    provenance TEXT NOT NULL,
    entry_json TEXT NOT NULL,
    updated_ts TEXT NOT NULL,
    UNIQUE(model_id, quant, hardware_profile, provenance)
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_benchmarks_model ON benchmarks(model_id, pillar);
CREATE INDEX IF NOT EXISTS idx_catalog_profile ON catalog_entries(hardware_profile);
"#;

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Db> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::configure(conn)
    }

    pub fn open_in_memory() -> anyhow::Result<Db> {
        Self::configure(Connection::open_in_memory()?)
    }

    fn configure(conn: Connection) -> anyhow::Result<Db> {
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        // WAL only meaningful on disk; in-memory stays on the memory journal.
        let mode: String = conn.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        if mode != "memory" {
            conn.pragma_update(None, "journal_mode", "WAL")?;
        }
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db { conn })
    }

    pub fn shared(self) -> SharedDb {
        Arc::new(Mutex::new(self))
    }

    // -- settings ----------------------------------------------------------

    pub fn set_setting(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> anyhow::Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        })
    }

    // -- hardware ----------------------------------------------------------

    pub fn insert_scan(&self, report: &HardwareReport) -> anyhow::Result<i64> {
        let json = serde_json::to_string(report)?;
        self.conn.execute(
            "INSERT INTO hardware_scans(ts, os, cpu_model, ram_total_mb, ram_free_mb, preferred_backend, tier, report_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![iso8601_now(), report.os, report.cpu_model, report.ram_total_mb, report.ram_free_mb, report.preferred_backend, report.tier, json],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn latest_scan(&self) -> anyhow::Result<Option<HardwareReport>> {
        let mut stmt = self
            .conn
            .prepare("SELECT report_json FROM hardware_scans ORDER BY id DESC LIMIT 1")?;
        let mut rows = stmt.query([])?;
        match rows.next()? {
            Some(row) => {
                let json: String = row.get(0)?;
                Ok(Some(serde_json::from_str(&json)?))
            }
            None => Ok(None),
        }
    }

    // -- discovered models ---------------------------------------------------

    pub fn upsert_model(&self, f: &crate::hf_crawler::DiscoveredFile) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO models(hf_repo, filename, quant, size_bytes, download_url, downloads, likes, trusted, discovered_ts)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(hf_repo, filename) DO UPDATE SET
               size_bytes = excluded.size_bytes,
               downloads = excluded.downloads,
               likes = excluded.likes,
               discovered_ts = excluded.discovered_ts",
            params![f.hf_repo, f.filename, f.quant, f.size_bytes, f.download_url, f.downloads, f.likes, f.trusted as i64, iso8601_now()],
        )?;
        Ok(())
    }

    pub fn models_for_query(&self, query: &str) -> anyhow::Result<Vec<serde_json::Value>> {
        let like = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT hf_repo, filename, quant, size_bytes, download_url, downloads, likes, trusted
             FROM models WHERE hf_repo LIKE ?1 OR filename LIKE ?1
             ORDER BY trusted DESC, downloads DESC LIMIT 100",
        )?;
        let rows = stmt.query_map(params![like], |row| {
            Ok(serde_json::json!({
                "hf_repo": row.get::<_, String>(0)?,
                "filename": row.get::<_, String>(1)?,
                "quant": row.get::<_, Option<String>>(2)?,
                "size_bytes": row.get::<_, i64>(3)?,
                "download_url": row.get::<_, String>(4)?,
                "downloads": row.get::<_, i64>(5)?,
                "likes": row.get::<_, i64>(6)?,
                "trusted": row.get::<_, i64>(7)? != 0,
            }))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // -- GGUF inspections ----------------------------------------------------

    pub fn insert_inspection(&self, info: &crate::gguf::GgufInfo) -> anyhow::Result<i64> {
        let json = serde_json::to_string(info)?;
        self.conn.execute(
            "INSERT INTO gguf_inspections(ts, path, architecture, quant, info_json) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![iso8601_now(), info.path, info.architecture, info.quant, json],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    // -- benchmarks ------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn insert_benchmark(
        &self,
        model_id: &str,
        hf_repo: &str,
        filename: &str,
        quant: &str,
        hardware_profile: &str,
        backend: &str,
        pillar: &str,
        score: f64,
        metrics: &serde_json::Value,
        provenance: &str,
    ) -> anyhow::Result<i64> {
        anyhow::ensure!(
            crate::catalog::PILLARS.contains(&pillar),
            "unknown pillar {pillar}"
        );
        anyhow::ensure!((0.0..=1.0).contains(&score), "score out of range");
        self.conn.execute(
            "INSERT INTO benchmarks(ts, model_id, hf_repo, filename, quant, hardware_profile, backend, pillar, score, metrics_json, provenance)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![iso8601_now(), model_id, hf_repo, filename, quant, hardware_profile, backend, pillar, score, metrics.to_string(), provenance],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_vibemark(
        &self,
        model_id: &str,
        backend: &str,
        engine: &str,
        prefill_tps: f64,
        decode_tps: f64,
        ttft_ms: f64,
        peak_ram_mb: f64,
        duration_s: f64,
    ) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO vibemark_runs(ts, model_id, backend, engine, prefill_tps, decode_tps, ttft_ms, peak_ram_mb, duration_s, provenance)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'measured')",
            params![iso8601_now(), model_id, backend, engine, prefill_tps, decode_tps, ttft_ms, peak_ram_mb, duration_s],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn latest_vibemark(&self) -> anyhow::Result<Option<serde_json::Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, model_id, backend, engine, prefill_tps, decode_tps, ttft_ms, peak_ram_mb, duration_s
             FROM vibemark_runs ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        Ok(match rows.next()? {
            Some(r) => Some(serde_json::json!({
                "ts": r.get::<_, String>(0)?,
                "model_id": r.get::<_, String>(1)?,
                "backend": r.get::<_, String>(2)?,
                "engine": r.get::<_, String>(3)?,
                "prefill_tps": r.get::<_, f64>(4)?,
                "decode_tps": r.get::<_, f64>(5)?,
                "ttft_ms": r.get::<_, f64>(6)?,
                "peak_ram_mb": r.get::<_, f64>(7)?,
                "duration_s": r.get::<_, f64>(8)?,
                "provenance": "measured",
            })),
            None => None,
        })
    }

    // -- catalog / leaderboard ---------------------------------------------------

    pub fn upsert_catalog_entry(&self, entry: &CatalogEntry) -> anyhow::Result<()> {
        entry.validate()?;
        let json = serde_json::to_string(entry)?;
        self.conn.execute(
            "INSERT INTO catalog_entries(model_id, quant, hardware_profile, provenance, entry_json, updated_ts)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(model_id, quant, hardware_profile, provenance) DO UPDATE SET
               entry_json = excluded.entry_json, updated_ts = excluded.updated_ts",
            params![entry.model_id, entry.quant, entry.hardware_profile, entry.provenance, json, iso8601_now()],
        )?;
        Ok(())
    }

    pub fn import_catalog(&self, catalog: &crate::catalog::Catalog) -> anyhow::Result<usize> {
        let mut n = 0;
        for entry in &catalog.entries {
            self.upsert_catalog_entry(entry)?;
            n += 1;
        }
        Ok(n)
    }

    pub fn leaderboard(&self, profile: &str) -> anyhow::Result<Vec<CatalogEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT entry_json FROM catalog_entries
             WHERE (?1 = '' OR hardware_profile = ?1)
             ORDER BY model_id, quant",
        )?;
        let rows = stmt.query_map(params![profile], |row| row.get::<_, String>(0))?;
        let mut entries = Vec::new();
        for row in rows {
            let entry: CatalogEntry = serde_json::from_str(&row?)?;
            entries.push(entry.with_derived());
        }
        // Measured/submitted entries supersede seed estimates for the same
        // (model, quant, profile) key: sort best-provenance first, then dedup.
        let rank = |p: &str| match p {
            "measured" => 2,
            "submitted" => 1,
            _ => 0,
        };
        entries.sort_by(|a, b| {
            a.model_id
                .cmp(&b.model_id)
                .then(a.quant.cmp(&b.quant))
                .then(a.hardware_profile.cmp(&b.hardware_profile))
                .then(rank(&b.provenance).cmp(&rank(&a.provenance)))
        });
        entries.dedup_by(|a, b| {
            a.model_id == b.model_id
                && a.quant == b.quant
                && a.hardware_profile == b.hardware_profile
        });
        Ok(entries)
    }

    pub fn benchmark_rows(&self, model_id: &str) -> anyhow::Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, pillar, score, metrics_json, provenance, hardware_profile, backend
             FROM benchmarks WHERE model_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![model_id], |row| {
            Ok(serde_json::json!({
                "ts": row.get::<_, String>(0)?,
                "pillar": row.get::<_, String>(1)?,
                "score": row.get::<_, f64>(2)?,
                "metrics": serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(3)?).unwrap_or(serde_json::Value::Null),
                "provenance": row.get::<_, String>(4)?,
                "hardware_profile": row.get::<_, Option<String>>(5)?,
                "backend": row.get::<_, Option<String>>(6)?,
            }))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{PillarScore, Throughput};
    use std::collections::BTreeMap;

    fn test_entry(model: &str, provenance: &str) -> CatalogEntry {
        let mut pillars = BTreeMap::new();
        for id in crate::catalog::PILLARS {
            pillars.insert(
                id.to_string(),
                PillarScore {
                    score: 0.7,
                    metric: "m".into(),
                    n: 5,
                    detail: serde_json::Value::Null,
                },
            );
        }
        CatalogEntry {
            model_id: model.into(),
            hf_repo: "bartowski/X-GGUF".into(),
            filename: "X-Q4_K_M.gguf".into(),
            download_url: "https://huggingface.co/bartowski/X-GGUF/resolve/main/X-Q4_K_M.gguf"
                .into(),
            quant: "Q4_K_M".into(),
            architecture: "qwen2".into(),
            family: "dense".into(),
            total_params_b: 7.6,
            active_params_b: 7.6,
            size_bytes: 4_680_000_000,
            context_length: 32768,
            hardware_profile: "rx580_vulkan".into(),
            provenance: provenance.into(),
            source_url: String::new(),
            pillars,
            throughput: Throughput::default(),
            vibe_quality_score: None,
            power_per_parameter: None,
            pareto_optimal: None,
        }
    }

    #[test]
    fn memory_db_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("token", "abc").unwrap();
        assert_eq!(db.get_setting("token").unwrap().as_deref(), Some("abc"));
        db.set_setting("token", "def").unwrap();
        assert_eq!(db.get_setting("token").unwrap().as_deref(), Some("def"));
        assert_eq!(db.get_setting("missing").unwrap(), None);
    }

    #[test]
    fn disk_db_wal_and_busy_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vibe_local.db");
        let db = Db::open(&path).unwrap();
        let mode: String = db
            .conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let timeout: i64 = db
            .conn
            .pragma_query_value(None, "busy_timeout", |r| r.get(0))
            .unwrap();
        assert_eq!(timeout, 5000);
        assert!(path.exists());
    }

    #[test]
    fn scan_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let report = crate::hardware::HardwareProbe::scan();
        db.insert_scan(&report).unwrap();
        let loaded = db.latest_scan().unwrap().unwrap();
        assert_eq!(loaded.ram_total_mb, report.ram_total_mb);
        assert_eq!(loaded.os, report.os);
    }

    #[test]
    fn benchmark_and_vibemark_persistence() {
        let db = Db::open_in_memory().unwrap();
        db.insert_benchmark(
            "M",
            "r",
            "f",
            "Q4_K_M",
            "rx580_vulkan",
            "vulkan",
            "code_generation",
            0.75,
            &serde_json::json!({"pass@1": 0.75}),
            "measured",
        )
        .unwrap();
        assert!(
            db.insert_benchmark(
                "M",
                "r",
                "f",
                "Q4_K_M",
                "p",
                "cpu",
                "not_a_pillar",
                0.5,
                &serde_json::Value::Null,
                "measured"
            )
            .is_err()
        );
        assert!(
            db.insert_benchmark(
                "M",
                "r",
                "f",
                "Q4_K_M",
                "p",
                "cpu",
                "code_generation",
                1.5,
                &serde_json::Value::Null,
                "measured"
            )
            .is_err()
        );
        let rows = db.benchmark_rows("M").unwrap();
        assert_eq!(rows.len(), 1);
        db.insert_vibemark(
            "M",
            "vulkan",
            "llama-server",
            500.0,
            30.0,
            120.0,
            5000.0,
            15.0,
        )
        .unwrap();
        let vm = db.latest_vibemark().unwrap().unwrap();
        assert_eq!(vm["decode_tps"], 30.0);
        assert_eq!(vm["provenance"], "measured");
    }

    #[test]
    fn leaderboard_provenance_supersede() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_catalog_entry(&test_entry("M", "seed_estimate"))
            .unwrap();
        db.upsert_catalog_entry(&test_entry("M", "measured"))
            .unwrap();
        db.upsert_catalog_entry(&test_entry("Other", "seed_estimate"))
            .unwrap();
        let board = db.leaderboard("rx580_vulkan").unwrap();
        assert_eq!(board.len(), 2);
        let m = board.iter().find(|e| e.model_id == "M").unwrap();
        assert_eq!(m.provenance, "measured");
        assert!(m.vibe_quality_score.is_some());
        // upsert replaces, not duplicates
        db.upsert_catalog_entry(&test_entry("M", "measured"))
            .unwrap();
        assert_eq!(db.leaderboard("rx580_vulkan").unwrap().len(), 2);
        // profile filter
        assert_eq!(db.leaderboard("tesla_t4").unwrap().len(), 0);
        assert_eq!(db.leaderboard("").unwrap().len(), 2);
    }

    #[test]
    fn invalid_entry_rejected() {
        let db = Db::open_in_memory().unwrap();
        let mut e = test_entry("M", "measured");
        e.family = "bogus".into();
        assert!(db.upsert_catalog_entry(&e).is_err());
    }

    #[test]
    fn model_upsert() {
        let db = Db::open_in_memory().unwrap();
        let f = crate::hf_crawler::DiscoveredFile {
            hf_repo: "bartowski/X-GGUF".into(),
            filename: "X-Q4_K_M.gguf".into(),
            quant: Some("Q4_K_M".into()),
            size_bytes: 100,
            download_url: "https://huggingface.co/bartowski/X-GGUF/resolve/main/X-Q4_K_M.gguf"
                .into(),
            downloads: 500,
            likes: 10,
            trusted: true,
        };
        db.upsert_model(&f).unwrap();
        db.upsert_model(&f).unwrap();
        let found = db.models_for_query("X").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["downloads"], 500);
    }
}
