use std::path::Path;

use apk_info::{Apk, ZipEntry};
use serde::Serialize;
use thiserror::Error;

/// Kesalahan loader APK.
///
/// Semua kegagalan input tidak tepercaya dipetakan ke varian eksplisit; tidak
/// ada kegagalan yang ditelan atau diganti dengan nilai default yang tampak
/// sukses.
#[derive(Debug, Error)]
pub enum LoaderError {
    #[error("tidak dapat membaca metadata: {0}")]
    Metadata(#[source] std::io::Error),
    #[error("bukan APK yang valid atau rusak: {0}")]
    InvalidApk(#[source] apk_info::APKError),
    #[error("tidak dapat membuka arsip APK: {0}")]
    Archive(#[source] apk_info::ZipError),
    #[error("tidak dapat membaca arsip dalam kontainer: {0}")]
    Container(#[source] apk_info::APKError),
    #[error("kompresi entry DEX tidak dikenal: {0}")]
    UnsupportedCompression(u16),
    #[error("entry DEX '{entry}' tidak ditemukan di APK")]
    DexNotFound { entry: String },
    #[error("gagal membaca DEX '{entry}': {source}")]
    DexRead {
        entry: String,
        #[source]
        source: apk_info::ZipError,
    },
    #[error("berkas DEX '{entry}' terlalu besar: {size} byte (batas {limit} byte)")]
    DexTooLarge {
        entry: String,
        size: u64,
        limit: u64,
    },
}

#[derive(Serialize)]
pub struct ApkReport {
    pub package_name: String,
    pub version_name: String,
    pub version_code: String,
    pub target_sdk: String,
    pub min_sdk: String,
    pub file_size: u64,
    pub mime_type: String,
    pub jar_seal: String,
    pub permissions: Vec<String>,
    pub features: Vec<String>,
    pub libraries: Vec<String>,
    pub native_libraries: Vec<String>,
    pub activities: Vec<String>,
    /// Activity yang dapat diluncurkan (intent filter MAIN + LAUNCHER).
    pub launchable_activities: Vec<String>,
    pub services: Vec<String>,
    pub receivers: Vec<String>,
    pub providers: Vec<String>,
    pub is_multidex: bool,
    pub debuggable: Option<String>,
    pub application_label: Option<String>,
    /// Pasangan `name`/`value` dari elemen `<meta-data>` pada `<application>`.
    ///
    /// Hanya pasangan yang lengkap yang dilaporkan; entri dengan
    /// `android:resource` atau jumlah atribut yang tidak seimbang dilewati
    /// agar runtime tidak menebak nilai.
    pub meta_data: Vec<(String, String)>,
    pub dex_entries: Vec<DexEntry>,
}

#[derive(Serialize, Clone)]
pub struct DexEntry {
    pub name: String,
    pub path: String,
    pub compression: String,
    pub size_compressed: u64,
    pub size_uncompressed: u64,
}

pub fn inspect_apk(path: &Path) -> Result<ApkReport, LoaderError> {
    let metadata = std::fs::metadata(path).map_err(LoaderError::Metadata)?;
    let file_size = metadata.len();

    let apk = Apk::new(path).map_err(LoaderError::InvalidApk)?;

    let package_name = apk
        .get_package_name()
        .unwrap_or_else(|| "tidak-diketahui".to_string());
    let version_name = apk.get_version_name().unwrap_or_default();
    let version_code = apk.get_version_code().unwrap_or_else(|| "0".to_string());
    let target_sdk = apk.get_target_sdk_version().to_string();
    let min_sdk = apk
        .get_min_sdk_version()
        .unwrap_or_else(|| "tidak-ditentukan".to_string());
    let permissions: Vec<String> = apk.get_permissions().map(|s| s.to_string()).collect();
    let features: Vec<String> = apk.get_features().map(|s| s.to_string()).collect();
    let libraries: Vec<String> = apk.get_libraries().map(|s| s.to_string()).collect();
    let native_libraries: Vec<String> = apk.get_native_libraries().map(|s| s.to_string()).collect();
    let activities: Vec<String> = apk
        .get_activities()
        .map(|a| a.name.map(|s| s.to_string()).unwrap_or_default())
        .collect();
    let launchable_activities: Vec<String> = apk
        .get_main_activities()
        .map(|name| name.to_string())
        .collect();
    let services: Vec<String> = apk
        .get_services()
        .map(|s| s.name.map(|s| s.to_string()).unwrap_or_default())
        .collect();
    let receivers: Vec<String> = apk
        .get_receivers()
        .map(|r| r.name.map(|s| s.to_string()).unwrap_or_default())
        .collect();
    let providers: Vec<String> = apk
        .get_providers()
        .map(|p| p.name.map(|s| s.to_string()).unwrap_or_default())
        .collect();

    let signature_count = apk.get_signatures().map(|s| s.len()).unwrap_or(0);

    let jar_seal = if signature_count == 0 {
        "tidak ada".to_string()
    } else {
        format!("{} tanda tangan", signature_count)
    };

    let is_multidex = apk.is_multidex();
    let debuggable = apk
        .get_application_debuggable()
        .map(|v| v.to_lowercase())
        .or_else(|| Some("tidak-diketahui".to_string()));
    let application_label = apk.get_application_label();
    let meta_data = meta_data_pairs(&apk);
    let dex_entries = dex_entries_for(&apk, path)?;

    Ok(ApkReport {
        package_name,
        version_name,
        version_code,
        target_sdk,
        min_sdk,
        file_size,
        mime_type: "application/vnd.android.package-archive".to_string(),
        jar_seal,
        permissions,
        features,
        libraries,
        native_libraries,
        activities,
        launchable_activities,
        services,
        receivers,
        providers,
        is_multidex,
        debuggable,
        application_label,
        meta_data,
        dex_entries,
    })
}

/// Pasangan `name`/`value` elemen `<meta-data>`.
///
/// Manifest dibaca ulang dan ditelusuri per elemen agar `name` dan `value`
/// tetap berpasangan meskipun ada entri yang memakai `android:resource`.
/// Entri tanpa `android:value` dilewati, bukan ditebak.
fn meta_data_pairs(apk: &Apk) -> Vec<(String, String)> {
    let Ok((manifest, _)) = apk.read(MANIFEST_ENTRY) else {
        return Vec::new();
    };
    let mut input = manifest.as_slice();
    let Ok(axml) = apk_info::AXML::new(&mut input, None) else {
        return Vec::new();
    };

    axml.root
        .descendants()
        .filter(|element| element.name() == "meta-data")
        .filter_map(|element| {
            let name = element.attr("name")?;
            let value = element.attr("value")?;
            let value = apk
                .get_resource_value(value)
                .unwrap_or_else(|| value.to_string());
            Some((name.to_string(), value))
        })
        .collect()
}

/// Batas ukuran satu berkas DEX yang dibaca ke memori.
/// APK adalah input tidak tepercaya; batas ini mencegah dekompresi yang
/// menghabiskan memori host.
pub const MAX_DEX_BYTES: u64 = 256 * 1024 * 1024;

/// Nama entry manifest yang menandai APK tunggal (bukan container).
const MANIFEST_ENTRY: &str = "AndroidManifest.xml";

/// `true` bila arsip ini APK tunggal, bukan container XAPK/APKM.
pub fn is_single_apk(apk: &Apk) -> bool {
    apk.namelist().any(|name| name == MANIFEST_ENTRY)
}

/// Arsip ZIP yang memuat DEX: arsip APK itu sendiri, atau `base.apk`
/// di dalam container.
enum DexArchive<'a> {
    Single(ZipEntry),
    Container(&'a ZipEntry),
}

impl DexArchive<'_> {
    fn entry(&self) -> &ZipEntry {
        match self {
            DexArchive::Single(zip) => zip,
            DexArchive::Container(zip) => zip,
        }
    }
}

fn open_dex_archive<'a>(apk: &'a Apk, path: &Path) -> Result<DexArchive<'a>, LoaderError> {
    if is_single_apk(apk) {
        let zip = ZipEntry::open(path).map_err(LoaderError::Archive)?;
        Ok(DexArchive::Single(zip))
    } else {
        let inner = apk.get_inner_zip().map_err(LoaderError::Container)?;
        Ok(DexArchive::Container(inner))
    }
}

/// Nama entry `classes*.dex` yang sah, mengikuti pola ART.
fn is_dex_entry_name(name: &str) -> bool {
    name.strip_prefix("classes")
        .and_then(|rest| rest.strip_suffix(".dex"))
        .is_some_and(|middle| middle.is_empty() || middle.bytes().all(|b| b.is_ascii_digit()))
}

/// Daftar berkas DEX di dalam APK (atau `base.apk` pada container).
/// Hanya membaca central directory; isi DEX tidak didekompresi.
fn dex_entries_for(apk: &Apk, path: &Path) -> Result<Vec<DexEntry>, LoaderError> {
    let archive = open_dex_archive(apk, path)?;
    let zip = archive.entry();

    let mut entries = Vec::new();
    for name in zip.namelist() {
        if !is_dex_entry_name(name) {
            continue;
        }
        let Some(info) = zip.entry_info(name) else {
            continue;
        };
        let compression = match info.compression_method {
            0 => "stored",
            8 => "deflated",
            other => return Err(LoaderError::UnsupportedCompression(other)),
        };
        entries.push(DexEntry {
            name: name.to_string(),
            path: name.to_string(),
            compression: compression.to_string(),
            size_compressed: info.compressed_size as u64,
            size_uncompressed: info.uncompressed_size as u64,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Membaca satu berkas DEX dari dalam APK sesuai nama entry-nya.
///
/// Isi DEX diperlakukan sebagai data tidak tepercaya: ukuran dibatasi
/// `MAX_DEX_BYTES` sebelum dan sesudah dekompresi.
pub fn dex_read_from_apk(path: &Path, entry_name: &str) -> Result<Vec<u8>, LoaderError> {
    let apk = Apk::new(path).map_err(LoaderError::InvalidApk)?;
    let archive = open_dex_archive(&apk, path)?;

    let info = archive
        .entry()
        .entry_info(entry_name)
        .ok_or_else(|| LoaderError::DexNotFound {
            entry: entry_name.to_string(),
        })?;
    if u64::from(info.uncompressed_size) > MAX_DEX_BYTES {
        return Err(LoaderError::DexTooLarge {
            entry: entry_name.to_string(),
            size: u64::from(info.uncompressed_size),
            limit: MAX_DEX_BYTES,
        });
    }

    let (bytes, _compression) =
        archive
            .entry()
            .read(entry_name)
            .map_err(|source| LoaderError::DexRead {
                entry: entry_name.to_string(),
                source,
            })?;
    if bytes.len() as u64 > MAX_DEX_BYTES {
        return Err(LoaderError::DexTooLarge {
            entry: entry_name.to_string(),
            size: bytes.len() as u64,
            limit: MAX_DEX_BYTES,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod dex_read_tests {
    use super::*;
    use arunika_test_fixtures::{AppSpec, apk_bytes_with_dex, apkm_bytes, minimal_dex};

    fn temp_apk(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("arunika-{}-{name}.apk", std::process::id()));
        std::fs::write(&path, bytes).expect("tidak dapat menulis fixture");
        path
    }

    #[test]
    fn dex_read_from_apk_membaca_classes_dex_dari_apk_biasa() {
        let dex = arunika_test_fixtures::header_only_dex(3, 1, 0);
        let path = temp_apk("plain-dex", &apk_bytes_with_dex(&AppSpec::default(), &dex));

        let bytes = dex_read_from_apk(&path, "classes.dex").expect("DEX harus terbaca");
        assert_eq!(bytes, dex);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dex_read_from_apk_membaca_dari_container_apkm() {
        let path = temp_apk("apkm-dex", &apkm_bytes(&AppSpec::default()));

        let bytes =
            dex_read_from_apk(&path, "classes.dex").expect("DEX di dalam base.apk harus terbaca");
        assert_eq!(bytes, minimal_dex());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dex_read_from_apk_menolak_entry_yang_tidak_ada() {
        let path = temp_apk(
            "missing-dex",
            &arunika_test_fixtures::apk_bytes(&AppSpec::default()),
        );

        let error =
            dex_read_from_apk(&path, "classes9.dex").expect_err("entry fiktif harus ditolak");
        assert!(
            matches!(error, LoaderError::DexNotFound { .. }),
            "varian error: {error:?}"
        );

        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::inspect_apk;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn temp(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("arunika-{}-{name}", std::process::id()));
        let _ = fs::remove_file(&path);
        fs::write(&path, bytes).expect("tidak dapat menulis file uji");
        path
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_non_zip_file() {
        let path = temp("nonzip.bin", b"bukan zip");
        assert!(inspect_apk(&path).is_err(), "file bukan zip harus ditolak");
        cleanup(&path);
    }

    #[test]
    fn rejects_empty_file() {
        let path = temp("empty.bin", b"");
        assert!(inspect_apk(&path).is_err(), "file kosong harus ditolak");
        cleanup(&path);
    }

    #[test]
    fn rejects_zip_without_manifest() {
        let path = temp(
            "no-manifest.zip",
            &arunika_test_fixtures::plain_zip_bytes(&[("beberapa.txt", b"tidak ada manifest")]),
        );
        assert!(
            inspect_apk(&path).is_err(),
            "manifest tidak ada harus ditolak"
        );
        cleanup(&path);
    }

    #[test]
    fn rejects_missing_file() {
        let path = std::env::temp_dir().join(format!("arunika-{}-missing.apk", std::process::id()));
        let _ = fs::remove_file(&path);
        assert!(
            inspect_apk(&path).is_err(),
            "berkas tidak ada harus ditolak"
        );
    }
}
