//! Pelapor crash untuk fase 2 MVP.
//!
//! Crate ini menyediakan model dan penyimpanan laporan crash (panic, eksepsi
//! tidak tertangkap, dll.) dalam bentuk terstruktur. Ia adalah crate domain
//! — tidak menangkap crash secara otomatis (tugas runtime/dex-engine) dan
//! tidak mengekspor crash ke host. Integrasi crash reporting ke host
//! dilakukan melalui `host-linux`/`host-macos` nanti.
//!
//! Laporan crash disimpan dalam memori dengan batas jumlah agar tidak tumbuh
//! tanpa batas.

use serde::Serialize;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Kesalahan pelapor crash.
#[derive(Debug, Error)]
pub enum CrashError {
    #[error("laporan crash tidak valid: {0}")]
    InvalidReport(String),
    #[error("penyimpanan crash penuh (maks {0} laporan)")]
    StorageFull(usize),
    #[error("laporan crash tidak ditemukan: {0}")]
    NotFound(u64),
}

/// Jenis crash yang dilaporkan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CrashType {
    /// Panic Rust.
    Panic,
    /// Eksepsi Java/DEX yang tidak tertangkap.
    UncaughtException,
    /// Kehabisan memori.
    OutOfMemory,
    /// Crash native (ELF/JNI).
    NativeCrash,
}

impl std::fmt::Display for CrashType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CrashType::Panic => write!(f, "panic"),
            CrashType::UncaughtException => write!(f, "uncaught-exception"),
            CrashType::OutOfMemory => write!(f, "out-of-memory"),
            CrashType::NativeCrash => write!(f, "native-crash"),
        }
    }
}

/// Satu laporan crash.
#[derive(Debug, Clone, Serialize)]
pub struct CrashReport {
    /// ID laporan (berurutan).
    pub id: u64,
    /// Waktu crash dalam detik sejak UNIX epoch.
    pub timestamp_secs: u64,
    /// Jenis crash.
    pub crash_type: CrashType,
    /// Pesan crash.
    pub message: String,
    /// Nama thread saat crash.
    pub thread: Option<String>,
    /// Konteks tambahan (paket, activity, dll.).
    pub context: HashMap<String, String>,
}

impl CrashReport {
    pub fn new(id: u64, crash_type: CrashType, message: impl Into<String>) -> Self {
        let timestamp_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            id,
            timestamp_secs,
            crash_type,
            message: message.into(),
            thread: None,
            context: HashMap::new(),
        }
    }

    /// Tambahkan pasangan kunci/nilai ke konteks.
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }

    /// Tetapkan nama thread.
    pub fn with_thread(mut self, thread: impl Into<String>) -> Self {
        self.thread = Some(thread.into());
        self
    }
}

/// Penyimpanan laporan crash dalam memori.
#[derive(Debug, Clone, Serialize, Default)]
pub struct CrashReporter {
    reports: Vec<CrashReport>,
    next_id: u64,
    /// Batas maksimal laporan yang disimpan.
    max_reports: usize,
}

impl CrashReporter {
    /// Buat pelapor crash dengan batas default (1000 laporan).
    pub fn new() -> Self {
        Self::with_max_reports(1000)
    }

    /// Buat pelapor dengan batas laporan tertentu.
    pub fn with_max_reports(max_reports: usize) -> Self {
        Self {
            max_reports,
            ..Self::default()
        }
    }

    /// Catat satu laporan crash. Kembalikan ID laporan.
    ///
    /// Jika penyimpanan penuh, laporan terbuang dihapus (FIFO) untuk memberi
    /// ruang pada yang baru.
    pub fn report(
        &mut self,
        crash_type: CrashType,
        message: impl Into<String>,
    ) -> Result<u64, CrashError> {
        let message = message.into();
        if message.is_empty() {
            return Err(CrashError::InvalidReport(
                "pesan crash tidak boleh kosong".into(),
            ));
        }
        if self.reports.len() >= self.max_reports {
            self.reports.remove(0);
        }
        let id = self.next_id;
        self.next_id += 1;
        let report = CrashReport::new(id, crash_type, message);
        self.reports.push(report);
        Ok(id)
    }

    /// Catat laporan crash dengan konteks dan thread.
    pub fn report_with_context(
        &mut self,
        crash_type: CrashType,
        message: impl Into<String>,
        thread: Option<impl Into<String>>,
        context: HashMap<String, String>,
    ) -> Result<u64, CrashError> {
        let mut report = CrashReport::new(self.next_id, crash_type, message);
        if let Some(t) = thread {
            report = report.with_thread(t.into());
        }
        report.context = context;
        let id = report.id;
        self.next_id += 1;
        if self.reports.len() >= self.max_reports {
            self.reports.remove(0);
        }
        self.reports.push(report);
        Ok(id)
    }

    /// Jumlah laporan yang tersimpan.
    pub fn len(&self) -> usize {
        self.reports.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reports.is_empty()
    }

    /// Ambil laporan berdasarkan ID.
    pub fn get(&self, id: u64) -> Option<&CrashReport> {
        self.reports.iter().find(|r| r.id == id)
    }

    /// Semua laporan, dari yang terlama ke terbaru.
    pub fn reports(&self) -> &[CrashReport] {
        &self.reports
    }

    /// Laporan terbaru, bila ada.
    pub fn latest(&self) -> Option<&CrashReport> {
        self.reports.last()
    }

    /// Hapus semua laporan.
    pub fn clear(&mut self) {
        self.reports.clear();
    }

    /// Batas maksimal laporan.
    pub fn max_reports(&self) -> usize {
        self.max_reports
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pelapor_awal_kosong() {
        let r = CrashReporter::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert!(r.latest().is_none());
    }

    #[test]
    fn laporkan_dan_dapatkan() {
        let mut r = CrashReporter::new();
        let id = r
            .report(CrashType::Panic, "out of bounds".to_string())
            .unwrap();
        assert_eq!(id, 0);
        assert_eq!(r.len(), 1);
        assert!(r.get(id).is_some());
        let report = r.get(id).unwrap();
        assert_eq!(report.crash_type, CrashType::Panic);
        assert_eq!(report.message, "out of bounds");
    }

    #[test]
    fn laporan_pesan_kosong_ditolak() {
        let mut r = CrashReporter::new();
        let err = r.report(CrashType::Panic, "".to_string()).unwrap_err();
        assert!(matches!(err, CrashError::InvalidReport { .. }));
    }

    #[test]
    fn laporan_dengan_konteks_dan_thread() {
        let mut r = CrashReporter::new();
        let mut ctx = HashMap::new();
        ctx.insert("package".to_string(), "com.contoh".to_string());
        ctx.insert("activity".to_string(), "MainActivity".to_string());
        let id = r
            .report_with_context(
                CrashType::UncaughtException,
                "null pointer",
                Some("main".to_string()),
                ctx,
            )
            .unwrap();
        let report = r.get(id).unwrap();
        assert_eq!(report.thread.as_deref(), Some("main"));
        assert_eq!(
            report.context.get("package"),
            Some(&"com.contoh".to_string())
        );
    }

    #[test]
    fn batas_laporan_dihapus_fifo() {
        let mut r = CrashReporter::with_max_reports(2);
        r.report(CrashType::Panic, "a".to_string()).unwrap();
        r.report(CrashType::Panic, "b".to_string()).unwrap();
        r.report(CrashType::Panic, "c".to_string()).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r.latest().unwrap().message, "c");
        assert!(r.get(0).is_none());
        assert!(r.get(1).is_some());
    }

    #[test]
    fn hapus_semua_laporan() {
        let mut r = CrashReporter::new();
        r.report(CrashType::Panic, "x".to_string()).unwrap();
        assert!(!r.is_empty());
        r.clear();
        assert!(r.is_empty());
        assert!(r.latest().is_none());
    }

    #[test]
    fn crash_report_serialisasi_json() {
        let report = CrashReport::new(1, CrashType::NativeCrash, "segfault")
            .with_context("paket", "com.contoh")
            .with_thread("main");
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("native-crash"));
        assert!(json.contains("segfault"));
        assert!(json.contains("com.contoh"));
    }

    #[test]
    fn crash_type_tampilkan() {
        assert_eq!(CrashType::Panic.to_string(), "panic");
        assert_eq!(
            CrashType::UncaughtException.to_string(),
            "uncaught-exception"
        );
        assert_eq!(CrashType::OutOfMemory.to_string(), "out-of-memory");
        assert_eq!(CrashType::NativeCrash.to_string(), "native-crash");
    }

    #[test]
    fn crash_error_tampilkan() {
        let e = CrashError::StorageFull(100);
        assert_eq!(e.to_string(), "penyimpanan crash penuh (maks 100 laporan)");
        let e2 = CrashError::NotFound(5);
        assert_eq!(e2.to_string(), "laporan crash tidak ditemukan: 5");
        let e3 = CrashError::InvalidReport("kosong".into());
        assert_eq!(e3.to_string(), "laporan crash tidak valid: kosong");
    }

    #[test]
    fn next_id_berlanjut() {
        let mut r = CrashReporter::new();
        let id1 = r.report(CrashType::Panic, "a".to_string()).unwrap();
        let id2 = r.report(CrashType::Panic, "b".to_string()).unwrap();
        assert_eq!(id1, 0);
        assert_eq!(id2, 1);
    }
}
