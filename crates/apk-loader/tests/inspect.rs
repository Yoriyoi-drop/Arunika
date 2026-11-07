use std::path::PathBuf;

use arunika_apk_loader::inspect_apk;
use arunika_test_fixtures::{
    AppSpec, apk_bytes, apk_bytes_with_entries, multidex_apk_bytes, xapk_bytes,
};

fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("arunika-inspect-{}-{name}", std::process::id()));
    std::fs::write(&path, bytes).expect("tidak dapat menulis fixture");
    path
}

#[test]
fn inspect_apk_membaca_metadata_manifest() {
    let spec = AppSpec {
        package: "com.arunika.fixture".to_string(),
        version_code: 42,
        version_name: "2.1.0".to_string(),
        min_sdk: 24,
        target_sdk: 35,
        label: Some("Aplikasi Uji".to_string()),
        debuggable: true,
        permissions: vec![
            "android.permission.INTERNET".to_string(),
            "android.permission.CAMERA".to_string(),
        ],
        features: vec!["android.hardware.camera".to_string()],
        activities: vec![".MainActivity".to_string(), ".SettingsActivity".to_string()],
        meta_data: vec![("com.arunika.channel".to_string(), "beta".to_string())],
    };
    let path = write_temp("metadata.apk", &apk_bytes(&spec));

    let report = inspect_apk(&path).expect("APK fixture harus dapat diinspeksi");
    assert_eq!(report.package_name, "com.arunika.fixture");
    assert_eq!(report.version_name, "2.1.0");
    assert_eq!(report.version_code, "42");
    assert_eq!(report.min_sdk, "24");
    assert_eq!(report.target_sdk, "35");
    assert_eq!(report.application_label.as_deref(), Some("Aplikasi Uji"));
    assert_eq!(report.debuggable.as_deref(), Some("true"));
    assert_eq!(report.activities.len(), 2);
    assert_eq!(report.permissions.len(), 2);
    assert_eq!(report.features, vec!["android.hardware.camera".to_string()]);
    assert_eq!(
        report.meta_data,
        vec![("com.arunika.channel".to_string(), "beta".to_string())]
    );
    assert_eq!(report.dex_entries.len(), 1);
    assert_eq!(report.dex_entries[0].name, "classes.dex");
    assert_eq!(report.dex_entries[0].compression, "stored");
    assert!(!report.is_multidex);
    assert_eq!(report.mime_type, "application/vnd.android.package-archive");

    let _ = std::fs::remove_file(path);
}

#[test]
fn inspect_apk_mendeteksi_multidex() {
    let path = write_temp("multidex.apk", &multidex_apk_bytes(&AppSpec::default()));

    let report = inspect_apk(&path).expect("APK multidex harus dapat diinspeksi");
    let names: Vec<&str> = report.dex_entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["classes.dex", "classes2.dex"]);
    assert!(report.is_multidex);

    let _ = std::fs::remove_file(path);
}

#[test]
fn inspect_apk_membaca_container_xapk() {
    let spec = AppSpec {
        package: "com.arunika.xapk".to_string(),
        ..AppSpec::default()
    };
    let path = write_temp("container.xapk", &xapk_bytes(&spec));

    let report = inspect_apk(&path).expect("XAPK fixture harus dapat diinspeksi");
    assert_eq!(report.package_name, "com.arunika.xapk");
    assert_eq!(report.dex_entries.len(), 1);
    assert_eq!(report.dex_entries[0].name, "classes.dex");

    let _ = std::fs::remove_file(path);
}

#[test]
fn inspect_apk_mengabaikan_entry_bukan_dex() {
    let path = write_temp(
        "noise.apk",
        &apk_bytes_with_entries(
            &AppSpec::default(),
            &[
                ("classes.dex", &arunika_test_fixtures::minimal_dex()),
                ("assets/classes.dex.bak", b"bukan dex"),
                ("classes.dex.txt", b"bukan dex"),
                ("resources.arsc", b"bukan arsc"),
            ],
        ),
    );

    let report = inspect_apk(&path).expect("entry non-DEX harus diabaikan");
    let names: Vec<&str> = report.dex_entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["classes.dex"]);

    let _ = std::fs::remove_file(path);
}
