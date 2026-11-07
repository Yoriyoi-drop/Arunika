//! Uji end-to-end CLI terhadap APK sintetis.
//!
//! Setiap tes memeriksa exit code dan bentuk keluaran yang dijanjikan
//! dokumentasi, bukan detail internal crate.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, ClassSpec, MethodSpec, build_dex, build_dex_with_symbols, insn,
};
use arunika_test_fixtures::{AppSpec, apk_bytes, apk_bytes_with_dex, apk_bytes_with_entries};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arunika"))
        .args(args)
        .output()
        .expect("gagal menjalankan biner arunika")
}

fn temp(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("arunika-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, bytes).expect("tidak dapat menulis fixture");
    path
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn json(output: &Output) -> serde_json::Value {
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout harus UTF-8");
    serde_json::from_str(&stdout).expect("keluaran harus JSON valid")
}

#[test]
fn help_keluar_dengan_sukses() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("arunika info"));
    assert!(text.contains("Aplikasi Android penuh belum didukung"));
    assert!(text.contains("arunika run"));
}

#[test]
fn info_json_memuat_metadata_dan_inspeksi_dex() {
    let spec = AppSpec {
        package: "com.arunika.cli".to_string(),
        version_name: "3.2.1".to_string(),
        ..AppSpec::default()
    };
    let path = temp("info.apk", &apk_bytes(&spec));

    let output = run(&["info", "--json", path.to_str().unwrap()]);
    assert!(output.status.success(), "info harus sukses");

    let payload = json(&output);
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["package_name"], "com.arunika.cli");
    assert_eq!(payload["version_name"], "3.2.1");
    assert_eq!(payload["dex"]["version"], "035");
    assert_eq!(payload["dex"]["header_size"], 112);
    assert_eq!(payload["dex_entries"][0]["name"], "classes.dex");
    assert!(payload.get("dex_error").is_none());

    cleanup(&path);
}

#[test]
fn info_tanpa_json_menampilkan_ringkasan() {
    let path = temp("info-text.apk", &apk_bytes(&AppSpec::default()));

    let output = run(&["info", path.to_str().unwrap()]);
    assert!(output.status.success());

    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("Paket:"));
    assert!(text.contains("com.example.fixture"));
    assert!(text.contains("DEX:"));
    assert!(text.contains("Versi DEX: 035"));

    cleanup(&path);
}

#[test]
fn info_tetap_sukses_saat_dex_rusak_dan_melaporkan_dex_error() {
    let path = temp(
        "broken-dex.apk",
        &apk_bytes_with_entries(&AppSpec::default(), &[("classes.dex", b"bukan dex")]),
    );

    let output = run(&["info", "--json", path.to_str().unwrap()]);
    assert!(output.status.success(), "metadata APK tetap harus tersedia");

    let payload = json(&output);
    assert_eq!(payload["package_name"], "com.example.fixture");
    let error = payload["dex_error"]
        .as_str()
        .expect("dex_error harus dilaporkan");
    assert!(error.contains("gagal menginspeksi DEX"), "pesan: {error}");

    cleanup(&path);
}

#[test]
fn compat_json_memuat_skema_dan_flag_kompatibilitas() {
    let spec = AppSpec {
        features: vec!["android.hardware.camera".to_string()],
        permissions: vec!["com.android.vending.BILLING".to_string()],
        ..AppSpec::default()
    };
    let path = temp("compat.apk", &apk_bytes(&spec));

    let output = run(&["compat", "--json", path.to_str().unwrap()]);
    assert!(output.status.success());

    let payload = json(&output);
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["dex_status"], "belum diuji");
    let codes: Vec<&str> = payload["flags"]
        .as_array()
        .expect("flags harus array")
        .iter()
        .filter_map(|flag| flag["code"].as_str())
        .collect();
    assert!(codes.contains(&"camera-required"));
    assert!(codes.contains(&"gms-or-billing"));

    cleanup(&path);
}

#[test]
fn doctor_memakai_laporan_yang_sama_dengan_compat() {
    let path = temp("doctor.apk", &apk_bytes(&AppSpec::default()));

    let compat = run(&["compat", "--json", path.to_str().unwrap()]);
    let doctor = run(&["doctor", "--json", path.to_str().unwrap()]);
    assert!(compat.status.success());
    assert!(doctor.status.success());
    assert_eq!(json(&compat), json(&doctor));

    cleanup(&path);
}

#[test]
fn info_pada_berkas_dex_langsung_bekerja() {
    let path = temp("langsung.dex", &arunika_test_fixtures::minimal_dex());

    let output = run(&["info", "--json", path.to_str().unwrap()]);
    assert!(output.status.success());

    let payload = json(&output);
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["cmd"], "info");
    assert_eq!(payload["version"], "035");

    cleanup(&path);
}

#[test]
fn berkas_tidak_ada_gagal_dengan_exit_1() {
    let output = run(&["info", "/tmp/arunika-tidak-ada.apk"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Gagal membaca APK"));
}

#[test]
fn perintah_tidak_dikenal_gagal_dengan_exit_2() {
    let output = run(&["install", "apa-saja.apk"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("belum tersedia"));
}

#[test]
fn opsi_tidak_dikenal_gagal_dengan_exit_2() {
    let output = run(&["info", "--format", "apa-saja.apk"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn dex_rusak_di_dalam_apk_tidak_menjatuhkan_cli() {
    let path = temp(
        "broken-dex-text.apk",
        &apk_bytes_with_dex(&AppSpec::default(), b"bukan dex"),
    );

    let output = run(&["info", path.to_str().unwrap()]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Peringatan DEX"));

    cleanup(&path);
}
#[test]
fn info_pada_container_xapk_menyertakan_inspeksi_dex() {
    let spec = AppSpec {
        package: "com.arunika.xapk.cli".to_string(),
        ..AppSpec::default()
    };
    let path = temp("container.xapk", &arunika_test_fixtures::xapk_bytes(&spec));

    let output = run(&["info", "--json", path.to_str().unwrap()]);
    assert!(output.status.success());

    let payload = json(&output);
    assert_eq!(payload["package_name"], "com.arunika.xapk.cli");
    assert_eq!(payload["dex"]["version"], "035");

    cleanup(&path);
}
#[cfg(unix)]
#[test]
fn stdout_tertutup_tidak_menyebabkan_panic() {
    use std::os::unix::process::ExitStatusExt;
    use std::process::Stdio;

    // Keluaran dibuat cukup besar agar melebihi buffer pipe dan memicu EPIPE.
    let activities: Vec<String> = (0..3000).map(|i| format!(".Activity{i}")).collect();
    let spec = AppSpec {
        activities,
        ..AppSpec::default()
    };
    let path = temp("sigpipe.apk", &apk_bytes(&spec));

    let mut child = Command::new(env!("CARGO_BIN_EXE_arunika"))
        .args(["info", path.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("gagal menjalankan biner arunika");

    drop(child.stdout.take());

    let output = child.wait_with_output().expect("gagal menunggu proses");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
    assert_eq!(
        output.status.signal(),
        Some(13),
        "proses harus berhenti karena SIGPIPE"
    );

    cleanup(&path);
}

fn dex_jawab() -> Vec<u8> {
    build_dex(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new("jawab", ACC_PUBLIC | ACC_STATIC, &[], "I")
                .registers(1)
                .ins(0)
                .body([insn::const_16(0, 42), insn::return_(0)].concat()),
        ),
    ])
}

#[test]
fn run_menjalankan_method_statis_di_apk() {
    let path = temp(
        "run.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex_jawab()),
    );
    let output = run(&[
        "run",
        "--class",
        "LContoh;",
        "--method",
        "jawab",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "stdout: {text}");
    assert!(text.contains("int 42"), "stdout: {text}");
    assert!(text.contains("Langkah:"), "stdout: {text}");
}

#[test]
fn run_json_memuat_langkah_dan_nilai() {
    let path = temp(
        "run-json.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex_jawab()),
    );
    let output = run(&[
        "run",
        "--json",
        "--class",
        "LContoh;",
        "--method",
        "jawab",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert!(output.status.success());
    let payload = json(&output);
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["return_type"], "I");
    assert_eq!(payload["return_value"]["kind"], "int");
    assert_eq!(payload["return_value"]["value"], 42);
    assert!(payload["steps"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn run_menerima_berkas_dex_langsung() {
    let path = temp("run.dex", &dex_jawab());
    let output = run(&[
        "run",
        "--class",
        "LContoh;",
        "--method",
        "jawab",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("int 42"));
}

#[test]
fn run_memakai_resolver_framework_untuk_math() {
    let build = |abs_index: u16| {
        let caller = MethodSpec::new("pakai", ACC_PUBLIC | ACC_STATIC, &[], "I")
            .registers(2)
            .ins(0)
            .body(
                [
                    insn::const_4(0, -5),
                    insn::invoke_static(abs_index, &[0]),
                    insn::move_result(1),
                    insn::return_(1),
                ]
                .concat(),
            );
        let math = MethodSpec::new("abs", ACC_PUBLIC | ACC_STATIC, &["I"], "I");
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(caller),
            ClassSpec::new("Ljava/lang/Math;", Some("Ljava/lang/Object;")).with_method(math),
        ])
    };

    let abs_index = build(0).method_index("Ljava/lang/Math;", "abs");
    let path = temp(
        "run-math.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &build(abs_index).bytes),
    );
    let output = run(&[
        "run",
        "--json",
        "--class",
        "LContoh;",
        "--method",
        "pakai",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert!(output.status.success());
    let payload = json(&output);
    assert_eq!(payload["return_value"]["value"], 5);
}

#[test]
fn run_method_tidak_ditemukan_gagal_dengan_kode_1() {
    let path = temp(
        "run-missing.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex_jawab()),
    );
    let output = run(&[
        "run",
        "--class",
        "LContoh;",
        "--method",
        "tidak_ada",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_tanpa_class_gagal_dengan_kode_2() {
    let output = run(&["run", "--method", "jawab", "a.apk"]);
    assert_eq!(output.status.code(), Some(2));
}

fn dex_activity() -> Vec<u8> {
    // Launcher activity mewarisi `android.app.Activity` dan memanggil
    // konstruktor induk seperti APK nyata.
    let build = |super_init: u16| {
        let activity = ClassSpec::new("Landroid/app/Activity;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V"));
        let main = ClassSpec::new(
            "Lcom/example/fixture/MainActivity;",
            Some("Landroid/app/Activity;"),
        )
        .with_method(
            MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V")
                .registers(1)
                .ins(1)
                .body([insn::invoke_direct(super_init, &[0]), insn::return_void()].concat()),
        )
        .with_method(
            MethodSpec::new("onCreate", ACC_PUBLIC, &["Landroid/os/Bundle;"], "V")
                .registers(2)
                .ins(2)
                .body(insn::return_void()),
        )
        .with_method(
            MethodSpec::new("onResume", ACC_PUBLIC, &[], "V")
                .registers(1)
                .ins(1)
                .body(insn::return_void()),
        );
        build_dex_with_symbols(&[activity, main])
    };

    let probe = build(0);
    build(probe.method_index("Landroid/app/Activity;", "<init>")).bytes
}
fn dex_view_activity() -> Vec<u8> {
    // Activity yang memanggil setContentView(0) agar runtime membuat
    // bitmap offscreen dan snapshot host dapat diuji.
    let build = |super_init: u16, set_content_view: u16| {
        let activity = ClassSpec::new("Landroid/app/Activity;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("setContentView", ACC_PUBLIC, &["I"], "V"))
            .with_method(MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V"));
        let main = ClassSpec::new(
            "Lcom/example/fixture/ViewActivity;",
            Some("Landroid/app/Activity;"),
        )
        .with_method(
            MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V")
                .registers(1)
                .ins(1)
                .body([insn::invoke_direct(super_init, &[0]), insn::return_void()].concat()),
        )
        .with_method(
            MethodSpec::new("onCreate", ACC_PUBLIC, &["Landroid/os/Bundle;"], "V")
                .registers(2)
                .ins(2)
                .body(
                    [
                        insn::const_4(1, 0),
                        insn::invoke_virtual(set_content_view, &[0, 1]),
                        insn::return_void(),
                    ]
                    .concat(),
                ),
        )
        .with_method(
            MethodSpec::new("onResume", ACC_PUBLIC, &[], "V")
                .registers(1)
                .ins(1)
                .body(insn::return_void()),
        );
        build_dex_with_symbols(&[activity, main])
    };

    let probe = build(0, 0);
    let super_init = probe.method_index("Landroid/app/Activity;", "<init>");
    let set_content_view = probe.method_index("Landroid/app/Activity;", "setContentView");
    build(super_init, set_content_view).bytes
}

fn dex_key_activity() -> Vec<u8> {
    // Activity yang mengganti dispatchKeyEvent dan mengembalikan true agar
    // runtime melaporkan event keyboard ditangani.
    let build = |super_init: u16| {
        let activity = ClassSpec::new("Landroid/app/Activity;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V"));
        let main = ClassSpec::new(
            "Lcom/example/fixture/KeyActivity;",
            Some("Landroid/app/Activity;"),
        )
        .with_method(
            MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V")
                .registers(1)
                .ins(1)
                .body([insn::invoke_direct(super_init, &[0]), insn::return_void()].concat()),
        )
        .with_method(
            MethodSpec::new("onCreate", ACC_PUBLIC, &["Landroid/os/Bundle;"], "V")
                .registers(2)
                .ins(2)
                .body(insn::return_void()),
        )
        .with_method(
            MethodSpec::new("onResume", ACC_PUBLIC, &[], "V")
                .registers(1)
                .ins(1)
                .body(insn::return_void()),
        )
        .with_method(
            MethodSpec::new(
                "dispatchKeyEvent",
                ACC_PUBLIC,
                &["Landroid/view/KeyEvent;"],
                "Z",
            )
            .registers(2)
            .ins(2)
            .body([insn::const_4(0, 1), insn::return_(0)].concat()),
        );
        build_dex_with_symbols(&[activity, main])
    };

    let probe = build(0);
    build(probe.method_index("Landroid/app/Activity;", "<init>")).bytes
}

#[test]
fn run_dengan_key_melaporkan_ditangani() {
    let spec = AppSpec {
        activities: vec![".KeyActivity".to_string()],
        ..Default::default()
    };
    let path = temp(
        "run-key.apk",
        &apk_bytes_with_dex(&spec, &dex_key_activity()),
    );
    let output = run(&[
        "run",
        "--json",
        "--key",
        "66",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert!(output.status.success());
    let payload = json(&output);
    assert_eq!(payload["key_events"], 1);
    assert_eq!(payload["key_handled"], true);
}

#[test]
fn run_dengan_snapshot_menulis_bmp_host() {
    let spec = AppSpec {
        activities: vec![".ViewActivity".to_string()],
        ..Default::default()
    };
    let path = temp(
        "run-snapshot.apk",
        &apk_bytes_with_dex(&spec, &dex_view_activity()),
    );
    let snapshot = temp("snapshot.bmp", b"");
    cleanup(&snapshot);

    let output = run(&[
        "run",
        "--json",
        "--snapshot",
        snapshot.to_str().expect("path harus UTF-8"),
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert!(output.status.success(), "run harus sukses");
    let bytes = std::fs::read(&snapshot).expect("snapshot harus ditulis");
    assert_eq!(&bytes[0..2], b"BM");
    assert!(bytes.len() > 54);
    cleanup(&snapshot);
}

#[test]
fn run_dengan_present_membuka_host_window() {
    if std::env::var("DISPLAY").is_err() {
        return;
    }
    let spec = AppSpec {
        activities: vec![".ViewActivity".to_string()],
        ..Default::default()
    };
    let path = temp(
        "run-present.apk",
        &apk_bytes_with_dex(&spec, &dex_view_activity()),
    );

    let output = run(&[
        "run",
        "--json",
        "--present",
        "--present-ms",
        "50",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert!(output.status.success(), "run harus sukses");
    let payload = json(&output);
    assert!(payload["window"]["present"].as_bool().unwrap_or(false));
}

#[test]
fn e2e_mcgg_apk_kompatibilitas_dan_laporan() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mcgg.apk");
    if !path.exists() {
        return;
    }
    let output = run(&["run", "--json", path.to_str().expect("path harus UTF-8")]);
    assert!(
        output.status.success(),
        "mcgg.apk harus dapat dijalankan: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(&output);
    assert_eq!(payload["mode"], "activity");
    assert_eq!(payload["package"], "com.mobilechess.mobapay");
    assert_eq!(
        payload["lifecycle"],
        serde_json::json!(["onCreate", "onStart", "onResume"])
    );
    assert_eq!(payload["main_loop_idle"], true);
    assert_eq!(payload["window"]["present"], true);
    assert_eq!(payload["crash_reports"], 0);
}

#[test]
fn run_tanpa_key_melaporkan_tidak_ditangani() {
    let spec = AppSpec {
        activities: vec![".KeyActivity".to_string()],
        ..Default::default()
    };
    let path = temp(
        "run-nokey.apk",
        &apk_bytes_with_dex(&spec, &dex_key_activity()),
    );
    let output = run(&["run", "--json", path.to_str().expect("path harus UTF-8")]);
    cleanup(&path);

    assert!(output.status.success());
    let payload = json(&output);
    assert_eq!(payload["key_events"], 0);
    assert_eq!(payload["key_handled"], false);
}

#[test]
fn run_tanpa_opsi_menyiapkan_launcher_activity() {
    let path = temp(
        "run-auto.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex_activity()),
    );
    let output = run(&["run", path.to_str().expect("path harus UTF-8")]);
    cleanup(&path);

    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "stdout: {text}");
    assert!(
        text.contains("Lcom/example/fixture/MainActivity;"),
        "stdout: {text}"
    );
    assert!(text.contains("Activity dibuat"), "stdout: {text}");
}

#[test]
fn run_tanpa_opsi_json_memuat_mode_activity() {
    let path = temp(
        "run-auto-json.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex_activity()),
    );
    let output = run(&["run", "--json", path.to_str().expect("path harus UTF-8")]);
    cleanup(&path);

    assert!(output.status.success());
    let payload = json(&output);
    assert_eq!(payload["mode"], "activity");
    assert_eq!(payload["activity"], ".MainActivity");
    assert_eq!(payload["class"], "Lcom/example/fixture/MainActivity;");
    assert_eq!(payload["instantiated"], true);
    assert_eq!(
        payload["lifecycle"],
        serde_json::json!(["onCreate", "onResume"])
    );
}

#[test]
fn run_tanpa_opsi_pada_apk_tanpa_launcher_gagal() {
    let mut spec = AppSpec::default();
    spec.activities.clear();
    let path = temp(
        "run-no-launcher.apk",
        &apk_bytes_with_dex(&spec, &dex_activity()),
    );
    let output = run(&["run", path.to_str().expect("path harus UTF-8")]);
    cleanup(&path);

    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_tanpa_opsi_pada_berkas_dex_gagal_dengan_kode_2() {
    let path = temp("run-auto.dex", &dex_jawab());
    let output = run(&["run", path.to_str().expect("path harus UTF-8")]);
    cleanup(&path);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn run_meneruskan_argumen_sesuai_tipe_parameter() {
    let dex = build_dex(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new("tambah", ACC_PUBLIC | ACC_STATIC, &["I", "I"], "I")
                .registers(2)
                .ins(2)
                .body([insn::add_int(0, 0, 1), insn::return_(0)].concat()),
        ),
    ]);
    let path = temp(
        "run-args.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex),
    );
    let output = run(&[
        "run",
        "--json",
        "--class",
        "LContoh;",
        "--method",
        "tambah",
        "--args",
        "20,22",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(&output);
    assert_eq!(payload["return_value"]["value"], 42);
}

#[test]
fn run_argumen_dengan_jumlah_salah_gagal_dengan_kode_2() {
    let path = temp(
        "run-args-salah.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex_jawab()),
    );
    let output = run(&[
        "run",
        "--class",
        "LContoh;",
        "--method",
        "jawab",
        "--args",
        "1",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn run_tanpa_opsi_menjalankan_lifecycle_yang_ada() {
    let path = temp(
        "run-auto-lifecycle.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex_activity()),
    );
    let output = run(&["run", path.to_str().expect("path harus UTF-8")]);
    cleanup(&path);

    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "stdout: {text}");
    assert!(
        text.contains("Lifecycle:    onCreate, onResume"),
        "stdout: {text}"
    );
}

#[test]
fn run_tanpa_opsi_pada_activity_yang_tidak_ada_di_dex_gagal() {
    let spec = AppSpec {
        activities: vec![".TidakAda".to_string()],
        ..AppSpec::default()
    };
    let path = temp(
        "run-auto-missing-activity.apk",
        &apk_bytes_with_dex(&spec, &dex_activity()),
    );
    let output = run(&["run", path.to_str().expect("path harus UTF-8")]);
    cleanup(&path);

    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("tidak ditemukan di DEX"), "stderr: {text}");
}

#[test]
fn run_method_di_classes2_multidex() {
    // Method yang hanya ada di classes2.dex harus masih dapat dipanggil.
    let dex2 = build_dex(&[
        ClassSpec::new("LContoh2;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new("dua", ACC_PUBLIC | ACC_STATIC, &[], "I")
                .registers(1)
                .ins(0)
                .body([insn::const_16(0, 99), insn::return_(0)].concat()),
        ),
    ]);
    let path = temp(
        "run-multidex.apk",
        &apk_bytes_with_entries(
            &AppSpec::default(),
            &[("classes.dex", &dex_jawab()), ("classes2.dex", &dex2)],
        ),
    );
    let output = run(&[
        "run",
        "--json",
        "--class",
        "LContoh2;",
        "--method",
        "dua",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(&output);
    assert_eq!(payload["return_value"]["value"], 99);
}

#[test]
fn run_argumen_string_dan_long() {
    let dex = build_dex(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new(
                "ucapkan",
                ACC_PUBLIC | ACC_STATIC,
                &["Ljava/lang/String;", "J"],
                "I",
            )
            .registers(3)
            .ins(3)
            .body([insn::const_4(0, 7), insn::return_(0)].concat()),
        ),
    ]);
    let path = temp(
        "run-arg-types.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex),
    );
    let output = run(&[
        "run",
        "--json",
        "--class",
        "LContoh;",
        "--method",
        "ucapkan",
        "--args",
        "hai,1234567890123",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert!(output.status.success());
    let payload = json(&output);
    assert_eq!(payload["return_value"]["value"], 7);
}

#[test]
fn run_argumen_boolean() {
    let dex = build_dex(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new("cek", ACC_PUBLIC | ACC_STATIC, &["Z"], "I")
                .registers(1)
                .ins(1)
                .body([insn::const_4(0, 1), insn::return_(0)].concat()),
        ),
    ]);
    let path = temp(
        "run-arg-bool.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex),
    );
    let output = run(&[
        "run",
        "--json",
        "--class",
        "LContoh;",
        "--method",
        "cek",
        "--args",
        "true",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert!(output.status.success());
    let payload = json(&output);
    assert_eq!(payload["return_value"]["value"], 1);
}

#[test]
fn run_argumen_tipe_salah_gagal_dengan_kode_2() {
    let path = temp(
        "run-arg-bad-type.apk",
        &apk_bytes_with_dex(&AppSpec::default(), &dex_jawab()),
    );
    let output = run(&[
        "run",
        "--class",
        "LContoh;",
        "--method",
        "jawab",
        "--args",
        "bukan-angka",
        path.to_str().expect("path harus UTF-8"),
    ]);
    cleanup(&path);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn run_apk_bukan_zip_gagal_dengan_kode_1() {
    let path = temp("run-notzip.apk", b"bukan zip sama sekali");
    let output = run(&["run", path.to_str().expect("path harus UTF-8")]);
    cleanup(&path);

    assert_eq!(output.status.code(), Some(1));
}
