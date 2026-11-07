use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("kebijakan isolasi tidak dapat diterapkan: {0}")]
    PolicyUnapplied(String),
    #[error("sandbox gagal disiapkan: {0}")]
    SetupFailed(String),
    #[error("izin yang diminta ditolak oleh kebijakan: {0}")]
    Denied(String),
    #[error("nama berkas keluar dari direktori aplikasi: {0}")]
    PathEscape(String),
    #[error("berkas '{name}' melebihi batas {limit} byte")]
    TooLarge { name: String, limit: usize },
    #[error("operasi berkas '{name}' gagal: {message}")]
    FileIo { name: String, message: String },
}

/// Kebijakan isolasi aplikasi Android.
///
/// Di fase 0, sandbox-manager mendefinisikan niat isolasi dan API
/// untuk menolak panggilan yang melanggar batas. Implementasi
/// isolasi sesungguhnya datang setelah model keamanan dan pengujian
/// isolasi siap.
#[derive(Debug, Clone, Serialize)]
pub struct SandboxPolicy {
    pub isolated_storage: bool,
    pub network_default_deny: bool,
    pub cross_app_data_access_denied: bool,
}

impl SandboxPolicy {
    pub fn default_strict() -> Self {
        Self {
            isolated_storage: true,
            network_default_deny: true,
            cross_app_data_access_denied: true,
        }
    }
}

/// Batas ukuran satu berkas yang boleh dibaca atau ditulis runtime.
///
/// APK adalah kode tidak tepercaya; tanpa batas ini satu berkas besar dapat
/// menghabiskan memori host.
pub const MAX_APP_FILE_BYTES: usize = 8 * 1024 * 1024;

/// Batas panjang nama berkas (batas nama berkas umum di Linux).
pub const MAX_APP_FILE_NAME: usize = 255;

/// Nama berkas relatif yang aman di dalam direktori aplikasi.
///
/// Hanya satu komponen nama biasa yang diterima: pemisah path, awalan
/// absolut, `.`, `..`, dan byte NUL ditolak sehingga kode aplikasi tidak
/// dapat keluar dari direktori datanya.
pub fn validate_file_name(name: &str) -> Result<&str, SandboxError> {
    if name.is_empty() {
        return Err(SandboxError::PathEscape("nama kosong".to_string()));
    }
    if name.len() > MAX_APP_FILE_NAME {
        return Err(SandboxError::PathEscape(format!(
            "nama melebihi {MAX_APP_FILE_NAME} byte"
        )));
    }
    if name.contains(['/', '\\', '\0']) {
        return Err(SandboxError::PathEscape(name.to_string()));
    }
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(name),
        _ => Err(SandboxError::PathEscape(name.to_string())),
    }
}

/// Direktori aplikasi yang sudah dikonfinemen: satu-satunya tempat berkas
/// aplikasi boleh berada.
#[derive(Debug, Clone)]
pub struct ConfinedRoot {
    root: PathBuf,
}

impl ConfinedRoot {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path absolut untuk `name`, atau galat bila nama keluar dari root.
    ///
    /// Symlink yang sudah ada di dalam root ditolak agar tidak menjadi jalan
    /// keluar tanpa perlu mengikuti tautan.
    pub fn resolve(&self, name: &str) -> Result<PathBuf, SandboxError> {
        validate_file_name(name)?;
        // Root yang berupa symlink juga merupakan jalan keluar, jadi tolak
        // sebelum nama berkas apa pun digabungkan.
        if let Ok(metadata) = std::fs::symlink_metadata(&self.root)
            && metadata.file_type().is_symlink()
        {
            return Err(SandboxError::PathEscape(self.root.display().to_string()));
        }
        let path = self.root.join(name);
        if let Ok(metadata) = std::fs::symlink_metadata(&path)
            && metadata.file_type().is_symlink()
        {
            return Err(SandboxError::PathEscape(name.to_string()));
        }
        Ok(path)
    }
}

/// Operasi berkas terbatas yang boleh dilakukan runtime atas nama aplikasi.
///
/// Kontrak ini sengaja sempit: hanya berkas langsung di dalam direktori data
/// aplikasi, tanpa direktori bersarang, tanpa symlink, dan dengan batas
/// ukuran. Implementasi platform berada di `arunika-host-linux`.
pub trait ConfinedFileSystem: Send + Sync + std::fmt::Debug {
    fn read(&self, name: &str) -> Result<Vec<u8>, SandboxError>;
    fn write(&self, name: &str, data: &[u8]) -> Result<(), SandboxError>;
    fn remove(&self, name: &str) -> Result<(), SandboxError>;
    fn list(&self) -> Result<Vec<String>, SandboxError>;
    fn exists(&self, name: &str) -> bool;
}

/// Kebijakan izin aplikasi yang dideklarasikan manifest.
///
/// Izin yang dideklarasikan dianggap diberikan saat pemasangan; izin lain
/// ditolak secara eksplisit. Model ini sengaja sederhana dan tidak menyediakan
/// dialog persetujuan runtime.
#[derive(Debug, Clone, Serialize, Default)]
pub struct PermissionPolicy {
    granted: std::collections::BTreeSet<String>,
}

impl PermissionPolicy {
    pub fn from_permissions(permissions: &[String]) -> Self {
        Self {
            granted: permissions.iter().cloned().collect(),
        }
    }

    pub fn is_granted(&self, permission: &str) -> bool {
        self.granted.contains(permission)
    }

    pub fn check(&self, permission: &str) -> Result<(), SandboxError> {
        if self.is_granted(permission) {
            Ok(())
        } else {
            Err(SandboxError::Denied(permission.to_string()))
        }
    }

    pub fn granted(&self) -> Vec<String> {
        self.granted.iter().cloned().collect()
    }

    pub fn grant(&mut self, permission: impl Into<String>) {
        self.granted.insert(permission.into());
    }

    pub fn revoke(&mut self, permission: &str) {
        self.granted.remove(permission);
    }
}

/// Batas panjang nama aset relatif di dalam APK.
pub const MAX_ASSET_NAME: usize = 1024;

/// Batas ukuran satu aset yang boleh dibaca runtime.
///
/// Aset ikut dibaca dari APK yang tidak tepercaya, jadi isinya tidak boleh
/// mengalokasikan memori host tanpa batas.
pub const MAX_ASSET_BYTES: usize = 16 * 1024 * 1024;

/// Nama aset relatif yang aman, boleh berupa subdirektori.
///
/// Berbeda dengan [`validate_file_name`], pemisah `/` diterima karena aset
/// memang bersarang (`config/level1.json`). Yang ditolak tetap sama: awalan
/// absolut, komponen `.`/`..`, nama kosong, byte NUL, dan nama kepanjangan.
pub fn validate_asset_name(name: &str) -> Result<&str, SandboxError> {
    if name.is_empty() {
        return Err(SandboxError::PathEscape("nama aset kosong".to_string()));
    }
    if name.len() > MAX_ASSET_NAME {
        return Err(SandboxError::PathEscape(format!(
            "nama aset melebihi {MAX_ASSET_NAME} byte"
        )));
    }
    if name.contains(['\\', '\0']) || name.starts_with('/') {
        return Err(SandboxError::PathEscape(name.to_string()));
    }
    let mut components = 0usize;
    for component in Path::new(name).components() {
        match component {
            Component::Normal(_) => components += 1,
            _ => return Err(SandboxError::PathEscape(name.to_string())),
        }
    }
    if components == 0 {
        return Err(SandboxError::PathEscape(name.to_string()));
    }
    Ok(name)
}

/// Anak langsung dari `prefix` pada daftar nama aset datar.
///
/// Semantiknya mengikuti `AssetManager.list(path)`: hanya nama satu tingkat di
/// bawah direktori yang diminta, dan subdirektori muncul sebagai nama tanpa
/// isinya. Dipakai bersama oleh semua implementasi [`AssetSource`] agar
/// perilakunya seragam.
pub fn asset_children<I: IntoIterator<Item = String>>(names: I, prefix: &str) -> Vec<String> {
    let base = prefix.trim_matches('/');
    let mut children = Vec::new();
    for name in names {
        let rest = if base.is_empty() {
            name.as_str()
        } else {
            match name
                .strip_prefix(base)
                .and_then(|rest| rest.strip_prefix('/'))
            {
                Some(rest) => rest,
                None => continue,
            }
        };
        if rest.is_empty() {
            continue;
        }
        if let Some(child) = rest.split('/').next() {
            children.push(child.to_string());
        }
    }
    children.sort();
    children.dedup();
    children
}

/// Sumber aset read-only milik aplikasi.
///
/// Aset berasal dari APK, bukan dari direktori data aplikasi, sehingga
/// kontraknya terpisah dari [`ConfinedFileSystem`]: hanya baca, tidak pernah
/// menulis, dan nama boleh bersarang. Implementasi platform berada di
/// `arunika-host-linux` (membaca entri `assets/` dari arsip APK).
pub trait AssetSource: Send + Sync + std::fmt::Debug {
    /// Isi aset, atau `None` bila aset tidak ada.
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, SandboxError>;

    /// Nama anak langsung di bawah `prefix` (kosong berarti akar aset).
    fn list(&self, prefix: &str) -> Result<Vec<String>, SandboxError>;

    /// Semua nama entri penuh (relatif terhadap prefiks sumber), terurut.
    fn entries(&self) -> Result<Vec<String>, SandboxError>;
}

/// [`AssetSource`] in-memory untuk pengujian dan untuk APK tanpa aset.
#[derive(Debug, Default, Clone)]
pub struct MemoryAssetSource {
    entries: std::collections::BTreeMap<String, Vec<u8>>,
}

impl MemoryAssetSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Menambahkan satu aset; nama dan ukuran divalidasi seperti sumber nyata.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Result<(), SandboxError> {
        let name = name.into();
        validate_asset_name(&name)?;
        let data = data.into();
        if data.len() > MAX_ASSET_BYTES {
            return Err(SandboxError::TooLarge {
                name,
                limit: MAX_ASSET_BYTES,
            });
        }
        self.entries.insert(name, data);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl AssetSource for MemoryAssetSource {
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, SandboxError> {
        validate_asset_name(name)?;
        Ok(self.entries.get(name).cloned())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, SandboxError> {
        Ok(asset_children(self.entries.keys().cloned(), prefix))
    }

    fn entries(&self) -> Result<Vec<String>, SandboxError> {
        let mut names: Vec<String> = self.entries.keys().cloned().collect();
        names.sort();
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AssetSource, ConfinedRoot, MAX_APP_FILE_NAME, MemoryAssetSource, PermissionPolicy,
        SandboxPolicy, validate_asset_name, validate_file_name,
    };

    #[test]
    fn kebijakan_baku_berisi_isolasi_kuatif() {
        let p = SandboxPolicy::default_strict();
        assert!(p.isolated_storage);
        assert!(p.network_default_deny);
        assert!(p.cross_app_data_access_denied);
    }

    #[test]
    fn nama_berkas_biasa_diterima() {
        assert_eq!(validate_file_name("cache.dat").ok(), Some("cache.dat"));
        assert_eq!(validate_file_name("a_b-c.1").ok(), Some("a_b-c.1"));
    }

    #[test]
    fn nama_berkas_keluar_root_ditolak() {
        for name in [
            "",
            ".",
            "..",
            "../rahasia",
            "sub/berkas",
            "/etc/passwd",
            "a\\b",
            "a\0b",
        ] {
            assert!(
                validate_file_name(name).is_err(),
                "nama {name:?} seharusnya ditolak"
            );
        }
    }

    #[test]
    fn nama_berkas_terlalu_panjang_ditolak() {
        let panjang = "a".repeat(MAX_APP_FILE_NAME + 1);
        assert!(validate_file_name(&panjang).is_err());
    }

    #[test]
    fn resolve_selalu_berada_di_bawah_root() {
        let root = ConfinedRoot::new("/tmp/arunika-uji");
        assert_eq!(
            root.resolve("data.bin").expect("nama sah"),
            std::path::Path::new("/tmp/arunika-uji/data.bin")
        );
        assert!(root.resolve("../luar.bin").is_err());
    }

    #[test]
    fn root_symlink_ditolak() {
        let root_path =
            std::env::temp_dir().join(format!("arunika-root-symlink-{}", std::process::id()));
        let target =
            std::env::temp_dir().join(format!("arunika-root-target-{}", std::process::id()));
        let _ = std::fs::remove_file(&root_path);
        let _ = std::fs::remove_dir_all(&target);
        std::fs::create_dir_all(&target).expect("target harus dibuat");
        std::os::unix::fs::symlink(&target, &root_path).expect("symlink harus dibuat");

        let root = ConfinedRoot::new(&root_path);
        assert!(
            root.resolve("data.bin").is_err(),
            "root yang berupa symlink harus ditolak"
        );

        let _ = std::fs::remove_file(&root_path);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn nama_aset_bersarang_diterima_dan_keluar_ditolak() {
        assert_eq!(
            validate_asset_name("config/level1.json").ok(),
            Some("config/level1.json")
        );
        assert_eq!(validate_asset_name("boot.bin").ok(), Some("boot.bin"));
        for name in [
            "",
            "/etc/passwd",
            "../luar.bin",
            "config/../../luar.bin",
            "./config",
            "a\\b",
            "a\0b",
        ] {
            assert!(
                validate_asset_name(name).is_err(),
                "nama aset {name:?} seharusnya ditolak"
            );
        }
    }

    #[test]
    fn memory_asset_source_membaca_dan_mendaftar_satu_tingkat() {
        let mut assets = MemoryAssetSource::new();
        assets
            .insert("boot.bin", b"halo".to_vec())
            .expect("aset sah");
        assets
            .insert("config/level1.json", b"{}".to_vec())
            .expect("aset bersarang sah");
        assets
            .insert("config/level2.json", b"{}".to_vec())
            .expect("aset bersarang sah");

        assert_eq!(
            assets.read("boot.bin").expect("baca").as_deref(),
            Some(&b"halo"[..])
        );
        assert!(assets.read("tidak-ada.bin").expect("baca").is_none());
        assert!(assets.read("../luar.bin").is_err());
        assert_eq!(
            assets.list("").expect("daftar akar"),
            vec!["boot.bin".to_string(), "config".to_string()]
        );
        assert_eq!(
            assets.list("config").expect("daftar config"),
            vec!["level1.json".to_string(), "level2.json".to_string()]
        );
    }

    #[test]
    fn permission_policy_menolak_izin_tidak_dideklarasikan() {
        let mut policy =
            PermissionPolicy::from_permissions(&["android.permission.INTERNET".to_string()]);
        assert!(policy.is_granted("android.permission.INTERNET"));
        assert!(!policy.is_granted("android.permission.CAMERA"));
        assert!(policy.check("android.permission.CAMERA").is_err());
        policy.grant("android.permission.CAMERA");
        assert!(policy.check("android.permission.CAMERA").is_ok());
        policy.revoke("android.permission.CAMERA");
        assert!(!policy.is_granted("android.permission.CAMERA"));
        assert_eq!(
            policy.granted(),
            vec!["android.permission.INTERNET".to_string()]
        );
    }
}
