use arunika_apk_loader::ApkReport;
use arunika_framework_api::FrameworkCoverageModel;
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct DoctorFlag {
    pub code: String,
    pub summary: String,
}

/// Versi skema keluaran JSON laporan kompatibilitas.
///
/// Pemanggil otomasi harus menolak skema yang tidak dikenal.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Clone)]
pub struct CompatibilityReport {
    pub schema_version: u32,
    pub package_name: String,
    pub dex_status: String,
    pub framework_status: String,
    pub native_status: String,
    pub framework_coverage: FrameworkCoverageModel,
    pub flags: Vec<DoctorFlag>,
    pub notes_lines: Vec<String>,
}

pub fn requires_camera_feature(features: &[String]) -> bool {
    features
        .iter()
        .any(|f| f.contains("android.hardware.camera"))
}

pub fn has_google_play_dependency(permissions: &[String]) -> bool {
    permissions.iter().any(|p| {
        p.contains("com.android.vending.BILLING")
            || p.contains("com.google.android.gms")
            || p.contains("com.google.android.finsky")
            || p.contains("com.google.android.c2dm")
    })
}

pub fn uses_legacy_library(libraries: &[String]) -> bool {
    libraries
        .iter()
        .any(|l| l.contains("org.apache.http.legacy"))
}

pub fn uses_native_library_named(libraries: &[String], needle: &str) -> bool {
    libraries.iter().any(|l| l.contains(needle))
}

impl CompatibilityReport {
    pub fn from_apk(report: &ApkReport) -> Self {
        let mut framework = FrameworkCoverageModel::empty();

        framework.declare_available("android.os", "Bundle");
        framework.declare_unavailable("android.content", "Intent");
        framework.declare_available("android.app", "Activity");
        framework.declare_available("android.view", "View");
        framework.declare_unavailable("android.widget", "TextView");
        framework.declare_available("android.graphics", "Canvas");
        framework.declare_available("android.graphics", "Bitmap");
        framework.declare_unavailable("native-bridge", "JNI");
        framework.declare_unavailable("native-bridge", "ELF");

        let mut notes_lines = Vec::new();
        let mut flags = Vec::new();

        let gms_or_billing = has_google_play_dependency(&report.permissions);
        let camera = requires_camera_feature(&report.features);

        if gms_or_billing {
            notes_lines.push(
                "APK meminta izin yang berkaitan dengan Google Play Billing atau GMS; layanan tersebut belum tersedia di runtime mandiri.".to_string(),
            );
            flags.push(DoctorFlag {
                code: "gms-or-billing".to_string(),
                summary: "Google Play Services / billing belum tersedia".to_string(),
            });
        }
        if camera {
            notes_lines.push(
                "APK merequiri fitur kamera; rendering dan view engine belum tersedia, sehingga jendela aplikasi belum dapat ditampilkan.".to_string(),
            );
            flags.push(DoctorFlag {
                code: "camera-required".to_string(),
                summary: "aplikasi merequiri kamera; view/graphics belum tersedia".to_string(),
            });
        }
        if uses_legacy_library(&report.libraries) {
            notes_lines.push("APK merequiri pustaka legacy (org.apache.http.legacy); runtime mandiri belum memetakannya.".to_string());
            flags.push(DoctorFlag {
                code: "legacy-library-required".to_string(),
                summary: "aplikasi merequiri pustaka legacy yang belum dipetakan".to_string(),
            });
        }
        if uses_native_library_named(&report.native_libraries, "vendor") {
            notes_lines.push(
                "APK merequiri pustaka native berlabel vendor; pemetaan native ABI belum tersedia."
                    .to_string(),
            );
            flags.push(DoctorFlag {
                code: "native-library-requires-mapping".to_string(),
                summary: "aplikasi merequiri pustaka native yang belum dipetakan".to_string(),
            });
        }
        if notes_lines.is_empty() {
            notes_lines.push("Fase 0: cakupan kompatibilitas belum tersedia".to_string());
        }

        Self {
            schema_version: SCHEMA_VERSION,
            package_name: report.package_name.clone(),
            dex_status: "belum diuji".to_string(),
            framework_status: "belum diuji".to_string(),
            native_status: "belum diuji".to_string(),
            framework_coverage: framework,
            flags,
            notes_lines,
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::has_google_play_dependency;
    use super::requires_camera_feature;

    #[test]
    fn detects_billing_permission() {
        let perms = vec!["com.android.vending.BILLING".to_string()];
        assert!(has_google_play_dependency(&perms));
    }

    #[test]
    fn detects_gms_permission() {
        let perms = vec!["com.google.android.gms.permission.AD_ID".to_string()];
        assert!(has_google_play_dependency(&perms));
    }

    #[test]
    fn detects_finsky_permission() {
        let perms = vec![
            "com.google.android.finsky.permission.BIND_GET_INSTALL_REFERRER_SERVICE".to_string(),
        ];
        assert!(has_google_play_dependency(&perms));
    }

    #[test]
    fn detects_c2dm_permission() {
        let perms = vec!["com.google.android.c2dm.permission.RECEIVE".to_string()];
        assert!(has_google_play_dependency(&perms));
    }

    #[test]
    fn ignores_plain_internet_permission() {
        let perms = vec!["android.permission.INTERNET".to_string()];
        assert!(!has_google_play_dependency(&perms));
    }

    #[test]
    fn ignores_empty_permissions() {
        let perms: Vec<String> = vec![];
        assert!(!has_google_play_dependency(&perms));
    }

    #[test]
    fn detects_camera_feature() {
        let features = vec!["android.hardware.camera".to_string()];
        assert!(requires_camera_feature(&features));
    }

    #[test]
    fn detects_camera_front_feature() {
        let features = vec!["android.hardware.camera.front".to_string()];
        assert!(requires_camera_feature(&features));
    }

    #[test]
    fn ignores_non_camera_features() {
        let features = vec!["android.hardware.location.gps".to_string()];
        assert!(!requires_camera_feature(&features));
    }

    #[test]
    fn ignores_empty_features() {
        let features: Vec<String> = vec![];
        assert!(!requires_camera_feature(&features));
    }

    #[test]
    fn flags_google_play_dependency() {
        let report = arunika_apk_loader::ApkReport {
            package_name: "pkg.test".to_string(),
            version_name: "1.0".to_string(),
            version_code: "1".to_string(),
            target_sdk: "34".to_string(),
            min_sdk: "21".to_string(),
            file_size: 100,
            mime_type: "application/vnd.android.package-archive".to_string(),
            jar_seal: "tidak ada".to_string(),
            permissions: vec!["com.android.vending.BILLING".to_string()],
            features: vec![],
            libraries: vec![],
            native_libraries: vec![],
            activities: vec![],
            launchable_activities: vec![],
            meta_data: vec![],
            services: vec![],
            receivers: vec![],
            providers: vec![],
            is_multidex: false,
            debuggable: Some("false".to_string()),
            application_label: None,
            dex_entries: vec![],
        };
        let r = super::CompatibilityReport::from_apk(&report);
        assert!(!r.flags.is_empty());
        assert!(r.flags.iter().any(|f| f.code == "gms-or-billing"));
    }

    #[test]
    fn flags_camera_feature() {
        let report = arunika_apk_loader::ApkReport {
            package_name: "pkg.test".to_string(),
            version_name: "1.0".to_string(),
            version_code: "1".to_string(),
            target_sdk: "34".to_string(),
            min_sdk: "21".to_string(),
            file_size: 100,
            mime_type: "application/vnd.android.package-archive".to_string(),
            jar_seal: "tidak ada".to_string(),
            permissions: vec![],
            features: vec!["android.hardware.camera".to_string()],
            libraries: vec![],
            native_libraries: vec![],
            activities: vec![],
            launchable_activities: vec![],
            meta_data: vec![],
            services: vec![],
            receivers: vec![],
            providers: vec![],
            is_multidex: false,
            debuggable: Some("false".to_string()),
            application_label: None,
            dex_entries: vec![],
        };
        let r = super::CompatibilityReport::from_apk(&report);
        assert!(!r.flags.is_empty());
        assert!(r.flags.iter().any(|f| f.code == "camera-required"));
    }

    #[test]
    fn no_flags_when_nothing_matched() {
        let report = arunika_apk_loader::ApkReport {
            package_name: "pkg.test".to_string(),
            version_name: "1.0".to_string(),
            version_code: "1".to_string(),
            target_sdk: "34".to_string(),
            min_sdk: "21".to_string(),
            file_size: 100,
            mime_type: "application/vnd.android.package-archive".to_string(),
            jar_seal: "tidak ada".to_string(),
            permissions: vec!["android.permission.INTERNET".to_string()],
            features: vec!["android.hardware.location.gps".to_string()],
            libraries: vec![],
            native_libraries: vec![],
            activities: vec![],
            launchable_activities: vec![],
            meta_data: vec![],
            services: vec![],
            receivers: vec![],
            providers: vec![],
            is_multidex: false,
            debuggable: Some("false".to_string()),
            application_label: None,
            dex_entries: vec![],
        };
        let r = super::CompatibilityReport::from_apk(&report);
        let has_flag = r
            .flags
            .iter()
            .any(|f| f.code == "gms-or-billing" || f.code == "camera-required");
        assert!(!has_flag);
    }

    #[test]
    fn detects_legacy_library() {
        let report = arunika_apk_loader::ApkReport {
            package_name: "pkg.test".to_string(),
            version_name: "1.0".to_string(),
            version_code: "1".to_string(),
            target_sdk: "34".to_string(),
            min_sdk: "21".to_string(),
            file_size: 100,
            mime_type: "application/vnd.android.package-archive".to_string(),
            jar_seal: "tidak ada".to_string(),
            permissions: vec![],
            features: vec![],
            libraries: vec!["org.apache.http.legacy".to_string()],
            native_libraries: vec![],
            activities: vec![],
            launchable_activities: vec![],
            meta_data: vec![],
            services: vec![],
            receivers: vec![],
            providers: vec![],
            is_multidex: false,
            debuggable: Some("false".to_string()),
            application_label: None,
            dex_entries: vec![],
        };
        let r = super::CompatibilityReport::from_apk(&report);
        assert!(r.flags.iter().any(|f| f.code == "legacy-library-required"));
    }

    #[test]
    fn ignores_non_legacy_libraries() {
        let report = arunika_apk_loader::ApkReport {
            package_name: "pkg.test".to_string(),
            version_name: "1.0".to_string(),
            version_code: "1".to_string(),
            target_sdk: "34".to_string(),
            min_sdk: "21".to_string(),
            file_size: 100,
            mime_type: "application/vnd.android.package-archive".to_string(),
            jar_seal: "tidak ada".to_string(),
            permissions: vec![],
            features: vec![],
            libraries: vec!["android.ext.adservices".to_string()],
            native_libraries: vec![],
            activities: vec![],
            launchable_activities: vec![],
            meta_data: vec![],
            services: vec![],
            receivers: vec![],
            providers: vec![],
            is_multidex: false,
            debuggable: Some("false".to_string()),
            application_label: None,
            dex_entries: vec![],
        };
        let r = super::CompatibilityReport::from_apk(&report);
        let has_flag = r.flags.iter().any(|f| f.code == "legacy-library-required");
        assert!(!has_flag);
    }

    #[test]
    fn detects_native_library_with_vendor_label() {
        let report = arunika_apk_loader::ApkReport {
            package_name: "pkg.test".to_string(),
            version_name: "1.0".to_string(),
            version_code: "1".to_string(),
            target_sdk: "34".to_string(),
            min_sdk: "21".to_string(),
            file_size: 100,
            mime_type: "application/vnd.android.package-archive".to_string(),
            jar_seal: "tidak ada".to_string(),
            permissions: vec![],
            features: vec![],
            libraries: vec![],
            native_libraries: vec!["com.example.vendor.lib".to_string()],
            activities: vec![],
            launchable_activities: vec![],
            meta_data: vec![],
            services: vec![],
            receivers: vec![],
            providers: vec![],
            is_multidex: false,
            debuggable: Some("false".to_string()),
            application_label: None,
            dex_entries: vec![],
        };
        let r = super::CompatibilityReport::from_apk(&report);
        assert!(
            r.flags
                .iter()
                .any(|f| f.code == "native-library-requires-mapping")
        );
    }

    #[test]
    fn ignores_native_library_without_vendor_label() {
        let report = arunika_apk_loader::ApkReport {
            package_name: "pkg.test".to_string(),
            version_name: "1.0".to_string(),
            version_code: "1".to_string(),
            target_sdk: "34".to_string(),
            min_sdk: "21".to_string(),
            file_size: 100,
            mime_type: "application/vnd.android.package-archive".to_string(),
            jar_seal: "tidak ada".to_string(),
            permissions: vec![],
            features: vec![],
            libraries: vec![],
            native_libraries: vec!["libcrashpad.so".to_string()],
            activities: vec![],
            launchable_activities: vec![],
            meta_data: vec![],
            services: vec![],
            receivers: vec![],
            providers: vec![],
            is_multidex: false,
            debuggable: Some("false".to_string()),
            application_label: None,
            dex_entries: vec![],
        };
        let r = super::CompatibilityReport::from_apk(&report);
        let has_flag = r
            .flags
            .iter()
            .any(|f| f.code == "native-library-requires-mapping");
        assert!(!has_flag);
    }

    #[test]
    fn compatibility_report_saat_ini_menyebut_framework_unavailable() {
        let report = arunika_apk_loader::ApkReport {
            package_name: "pkg.test".to_string(),
            version_name: "1.0".to_string(),
            version_code: "1".to_string(),
            target_sdk: "34".to_string(),
            min_sdk: "21".to_string(),
            file_size: 100,
            mime_type: "application/vnd.android.package-archive".to_string(),
            jar_seal: "tidak ada".to_string(),
            permissions: vec![],
            features: vec![],
            libraries: vec![],
            native_libraries: vec![],
            activities: vec![],
            launchable_activities: vec![],
            meta_data: vec![],
            services: vec![],
            receivers: vec![],
            providers: vec![],
            is_multidex: false,
            debuggable: Some("false".to_string()),
            application_label: None,
            dex_entries: vec![],
        };
        let r = super::CompatibilityReport::from_apk(&report);
        assert_for_coverage_unavailable(&r, "android.content", "Intent");
    }

    fn assert_for_coverage_unavailable(r: &crate::CompatibilityReport, group: &str, api: &str) {
        let group = r
            .framework_coverage
            .groups
            .iter()
            .find(|g| g.group == group)
            .unwrap();
        assert!(group.unavailable.contains(&api.to_owned()));
    }
}
