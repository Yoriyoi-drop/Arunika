use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("tidak dapat membaca manifes APK: {0}")]
    Io(#[from] std::io::Error),
    #[error("berkas manifes tidak ditemukan di APK")]
    Missing,
    #[error("berkas manifes tidak valid: {0}")]
    Parse(String),
}

/// Metadata tingkat tinggi yang diekstrak dari AndroidManifest.xml.
/// Ini adalah abstraksi domain, bukan representasi mentah XML.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestMetadata {
    pub package_name: String,
    pub version_name: String,
    pub version_code: String,
    pub target_sdk: String,
    pub min_sdk: String,
    pub application_label: Option<String>,
    pub debuggable: bool,
    pub permissions: Vec<String>,
    pub features: Vec<String>,
    pub uses_libraries: Vec<String>,
    pub activities: Vec<ActivityDecl>,
    pub services: Vec<ComponentDecl>,
    pub receivers: Vec<ComponentDecl>,
    pub providers: Vec<ComponentDecl>,
    pub is_multidex: bool,
    pub native_libraries: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivityDecl {
    pub name: String,
    pub enabled: bool,
    pub exported: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentDecl {
    pub name: String,
    pub enabled: bool,
    pub exported: Option<bool>,
}

impl ManifestMetadata {
    /// Placeholder implementasi fase 0.
    ///
    /// Saat ini manifest-parser tidak digunakan untuk inspeksi statis
    /// karena apk-loader sudah menyediakan arXiv metadata melalui apk-info.
    /// Setelah ekstraksi XML siap, fungsi ini akan menjadi renderer
    /// metadata mandiri untuk use case yang perlu representasi terstruktur
    /// tanpa bergantung pada implementasi APK loader spesifik.
    pub fn placeholder() -> Self {
        Self {
            package_name: "tidak-diketahui".into(),
            version_name: String::new(),
            version_code: "0".into(),
            target_sdk: String::new(),
            min_sdk: "tidak-ditentukan".into(),
            application_label: None,
            debuggable: false,
            permissions: Vec::new(),
            features: Vec::new(),
            uses_libraries: Vec::new(),
            activities: Vec::new(),
            services: Vec::new(),
            receivers: Vec::new(),
            providers: Vec::new(),
            is_multidex: false,
            native_libraries: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ManifestMetadata;

    #[test]
    fn placeholder_menghasilkan_metadata_default() {
        let m = ManifestMetadata::placeholder();
        assert_eq!(m.package_name, "tidak-diketahui");
        assert_eq!(m.version_name, "");
        assert_eq!(m.version_code, "0");
        assert_eq!(m.target_sdk, "");
        assert_eq!(m.min_sdk, "tidak-ditentukan");
        assert_eq!(m.application_label, None);
        assert!(!m.debuggable);
        assert!(m.permissions.is_empty());
        assert!(m.features.is_empty());
        assert!(m.uses_libraries.is_empty());
        assert!(m.activities.is_empty());
        assert!(m.services.is_empty());
        assert!(m.receivers.is_empty());
        assert!(m.providers.is_empty());
        assert!(!m.is_multidex);
        assert!(m.native_libraries.is_empty());
    }

    #[test]
    fn placeholder_dapat_diserialisasi_ke_json() {
        let m = ManifestMetadata::placeholder();
        let json = serde_json::to_string(&m).expect("placeholder harus dapat diserialisasi");
        assert!(json.contains("tidak-diketahui"));
        assert!(json.contains("\"debuggable\":false"));
    }

    #[test]
    fn metadata_klon_dan_debug() {
        let m = ManifestMetadata::placeholder();
        let cloned = m.clone();
        assert_eq!(cloned.package_name, m.package_name);
        let debug = format!("{m:?}");
        assert!(!debug.is_empty());
    }
}
