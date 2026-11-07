use arunika_apk_loader::{ApkReport, inspect_apk};
use arunika_compat_db::CompatibilityReport;
use arunika_crash_reporter::CrashType;
use arunika_dex_engine::{DexEngine, DexError, DexFile, DexInspection, Value};
use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

mod output;

fn path_looks_like_dex(path: &Path) -> bool {
    path.extension().map(|ext| ext == "dex").unwrap_or(false)
}

/// Versi skema keluaran JSON perintah info.
const INFO_SCHEMA_VERSION: u32 = 1;

fn json_error(error: serde_json::Error) -> (ExitCode, String) {
    (
        ExitCode::FAILURE,
        format!("Gagal membuat keluaran JSON: {error}"),
    )
}

const HELP: &str = "\
Arunika — inspeksi APK dan interpreter DEX subset (fase 1).
Aplikasi Android penuh belum didukung; hanya method statis yang dapat dijalankan.

Penggunaan:
  arunika --help
  arunika info   [--json] <file.apk|file.xapk|file.apkm|file.dex>
  arunika compat [--json] <file.apk|file.xapk|file.apkm>
  arunika doctor [--json] <file.apk|file.xapk|file.apkm>
  arunika run    [--json] [--key <kode>]... [--snapshot <path>] [--present] [--present-ms <ms>] <file.apk|file.xapk|file.apkm>
  arunika run    [--json] [--args <nilai,nilai>] --class <descriptor> --method <nama> <berkas>

Opsi --key (boleh diulang) memasukkan event keyboard secara terprogram ke
activity (kode tombol Android, mis. 66=ENTER, 4=BACK).
Opsi --snapshot <path> menyimpan snapshot jendela offscreen ke BMP.
Opsi --present membuka jendela host nyata dan menampilkan frame.

Tanpa opsi tambahan, run memuat APK dan menyiapkan launcher activity-nya.
Dengan --class/--method, run menjalankan satu method statis di interpreter DEX
subset; --args mengisi parameter sesuai tipe deklarasi method.
Perintah install/stop/uninstall belum tersedia.
";

/// Kembalikan perilaku Unix standar untuk SIGPIPE.
///
/// Tanpa ini, menulis ke stdout yang sudah ditutup (mis. `arunika info x.apk
/// | head`) memicu panic "failed printing to stdout". CLI Unix konvensional
/// berhenti karena sinyal, bukan panic.
#[cfg(unix)]
fn restore_default_sigpipe() {
    // SAFETY: mengembalikan disposisi sinyal ke default adalah operasi
    // process-wide yang aman dilakukan sebelum thread lain dibuat.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_default_sigpipe() {}

fn main() -> ExitCode {
    restore_default_sigpipe();
    match execute(std::env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, message)) => {
            eprintln!("{message}");
            code
        }
    }
}

fn execute(args: impl Iterator<Item = OsString>) -> Result<(), (ExitCode, String)> {
    let mut args = args;
    let Some(command) = args.next() else {
        print!("{HELP}");
        return Ok(());
    };

    let command = command.into_string().map_err(|_| {
        (
            ExitCode::from(2),
            format!("Perintah tidak valid.\n\n{HELP}"),
        )
    })?;
    if matches!(command.as_str(), "--help" | "-h" | "help") {
        if args.next().is_some() {
            return Err((ExitCode::from(2), format!("Argumen berlebih.\n\n{HELP}")));
        }
        print!("{HELP}");
        return Ok(());
    }

    if command == "run" {
        return run_method(args);
    }

    let remaining = args.collect::<Vec<_>>();
    if remaining.is_empty() || remaining.len() > 2 {
        return Err((
            ExitCode::from(2),
            format!("Perintah '{command}' memerlukan path APK.\n\n{HELP}"),
        ));
    }

    let (path, json) = match remaining.as_slice() {
        [path] => (path, false),
        [option, path] if option == "--json" => (path, true),
        [path, option] if option == "--json" => (path, true),
        _ => return Err((ExitCode::from(2), format!("Opsi tidak dikenal.\n\n{HELP}"))),
    };

    match command.as_str() {
        "info" => {
            if path_looks_like_dex(Path::new(path)) {
                return inspect_dex(path, json);
            }
            let report = load_report(path)?;
            let dex_result = inspect_apk_dex(path, &report);
            if json {
                let mut payload = serde_json::to_value(&report).map_err(json_error)?;
                if let Some(object) = payload.as_object_mut() {
                    object.insert("schema_version".to_owned(), INFO_SCHEMA_VERSION.into());
                    match &dex_result {
                        Ok(Some(apk_dex)) => {
                            object.insert(
                                "dex".to_owned(),
                                serde_json::to_value(apk_dex).map_err(json_error)?,
                            );
                        }
                        Ok(None) => {}
                        Err(message) => {
                            object.insert(
                                "dex_error".to_owned(),
                                serde_json::Value::String(message.clone()),
                            );
                        }
                    }
                }
                let serialized = serde_json::to_string_pretty(&payload).map_err(json_error)?;
                println!("{serialized}");
                Ok(())
            } else {
                output::print_info(&report);
                match dex_result {
                    Ok(Some(apk_dex)) => {
                        println!();
                        println!("DEX:");
                        print_dal_session(&apk_dex);
                    }
                    Ok(None) => {}
                    Err(message) => eprintln!("Peringatan DEX: {message}"),
                }
                Ok(())
            }
        }
        "compat" | "doctor" => {
            let apk_report = load_report(path)?;
            let report = CompatibilityReport::from_apk(&apk_report);
            if json {
                let serialized = report.to_json().map_err(|error| {
                    (
                        ExitCode::FAILURE,
                        format!("Gagal membuat laporan JSON: {error}"),
                    )
                })?;
                println!("{serialized}");
                Ok(())
            } else {
                output::print_compatibility(&report);
                Ok(())
            }
        }
        _ => Err((
            ExitCode::from(2),
            format!("Perintah '{command}' belum tersedia.\n\n{HELP}"),
        )),
    }
}

/// Skema keluaran JSON perintah `run`.
const RUN_SCHEMA_VERSION: u32 = 1;

/// Hasil perintah CLI beserta kode keluar saat gagal.
type CliResult<T> = Result<T, (ExitCode, String)>;

fn bad_usage(message: &str) -> (ExitCode, String) {
    (ExitCode::from(2), format!("{message}\n\n{HELP}"))
}

struct RunRequest {
    path: String,
    class: Option<String>,
    method: Option<String>,
    args: Option<String>,
    json: bool,
    /// Kode tombol yang dimasukkan secara terprogram (--key, boleh diulang).
    keys: Vec<i32>,
    /// Path BMP untuk menyimpan snapshot jendela offscreen setelah run.
    snapshot: Option<String>,
    /// Apakah jendela host nyata harus dibuka untuk presentasi frame.
    present: bool,
    /// Durasi presentasi host dalam milidetik.
    present_ms: u64,
}

/// Nilai argumen yang sudah diketik sesuai parameter method.
enum ArgValue {
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Text(String),
}

impl ArgValue {
    fn materialize(self, heap: &mut arunika_dex_engine::Heap) -> Value {
        match self {
            ArgValue::Int(value) => Value::Int(value),
            ArgValue::Long(value) => Value::from_long(value),
            ArgValue::Float(value) => Value::from_float(value),
            ArgValue::Double(value) => Value::from_double(value),
            ArgValue::Text(text) => heap.alloc(arunika_dex_engine::Object::String(text)),
        }
    }
}

fn next_value(
    args: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> Result<String, (ExitCode, String)> {
    args.next()
        .ok_or_else(|| bad_usage(&format!("Opsi {flag} membutuhkan nilai.")))?
        .into_string()
        .map_err(|_| bad_usage(&format!("Nilai {flag} bukan teks UTF-8.")))
}

fn parse_key_code(text: &str) -> Result<i32, (ExitCode, String)> {
    text.parse::<i32>()
        .map_err(|_| bad_usage(&format!("Kode tombol tidak valid: {text}")))
}

fn parse_run_args(
    mut args: impl Iterator<Item = OsString>,
) -> Result<RunRequest, (ExitCode, String)> {
    let mut json = false;
    let mut class = None;
    let mut method = None;
    let mut call_args = None;
    let mut keys: Vec<i32> = Vec::new();
    let mut snapshot: Option<String> = None;
    let mut present = false;
    let mut present_ms: u64 = 250;
    let mut path: Option<String> = None;

    while let Some(arg) = args.next() {
        let text = arg
            .into_string()
            .map_err(|_| bad_usage("Argumen bukan teks UTF-8."))?;
        match text.as_str() {
            "--json" => json = true,
            "--class" => class = Some(next_value(&mut args, "--class")?),
            "--method" => method = Some(next_value(&mut args, "--method")?),
            "--args" => call_args = Some(next_value(&mut args, "--args")?),
            "--key" => {
                let raw = next_value(&mut args, "--key")?;
                keys.push(parse_key_code(&raw)?);
            }
            "--snapshot" => {
                snapshot = Some(next_value(&mut args, "--snapshot")?);
            }
            "--present" => present = true,
            "--present-ms" => {
                let raw = next_value(&mut args, "--present-ms")?;
                present_ms = raw
                    .parse::<u64>()
                    .map_err(|_| bad_usage("Durasi presentasi tidak valid."))?;
            }
            other if other.starts_with("--") => {
                return Err(bad_usage(&format!("Opsi tidak dikenal: {other}")));
            }
            other => {
                if path.is_some() {
                    return Err(bad_usage("Hanya satu berkas yang boleh diberikan."));
                }
                path = Some(other.to_owned());
            }
        }
    }

    Ok(RunRequest {
        path: path.ok_or_else(|| bad_usage("Path berkas wajib diisi."))?,
        class,
        method,
        args: call_args,
        json,
        keys,
        snapshot,
        present,
        present_ms,
    })
}

/// Direktori data virtual per paket; belum dipakai untuk I/O disk.
fn app_data_dir(package: &str) -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".local").join("share"))
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("arunika")
        .join("apps")
        .join(sanitize_package(package))
}

/// Metadata manifest yang dilaporkan `PackageManager`.
///
/// Nilai yang tidak terbaca sebagai angka dibiarkan kosong, bukan ditebak.
fn manifest_metadata(report: &arunika_apk_loader::ApkReport) -> arunika_framework_api::AppManifest {
    arunika_framework_api::AppManifest {
        package: Some(report.package_name.clone()),
        version_name: Some(report.version_name.clone()),
        version_code: report.version_code.trim().parse::<i64>().ok(),
        target_sdk: report.target_sdk.trim().parse::<i32>().ok(),
        min_sdk: report.min_sdk.trim().parse::<i32>().ok(),
        label: report.application_label.clone(),
        meta_data: report.meta_data.clone(),
        permissions: report.permissions.clone(),
    }
}

/// Nama paket hanya boleh berisi karakter aman agar tidak keluar dari root.
fn sanitize_package(package: &str) -> String {
    let cleaned: String = package
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.starts_with('.') {
        "unknown".to_string()
    } else {
        cleaned
    }
}

/// Lifecycle yang dijalankan setelah activity dibuat, berurutan.
const LIFECYCLE_METHODS: [(&str, &[&str]); 3] = [
    ("onCreate", &["Landroid/os/Bundle;"]),
    ("onStart", &[]),
    ("onResume", &[]),
];

/// Apakah class memiliki method dengan proto persis seperti yang diminta.
fn has_method(dex: &DexFile, class: &str, name: &str, parameters: &[&str]) -> bool {
    dex.method_ids.iter().any(|method| {
        method.class == class
            && method.name == name
            && dex
                .proto_ids
                .get(method.proto_idx as usize)
                .is_some_and(|proto| proto.parameter_types == parameters)
    })
}

/// Ubah nama activity dari manifest menjadi descriptor DEX.
fn activity_descriptor(name: &str, package: &str) -> String {
    let qualified = if let Some(rest) = name.strip_prefix('.') {
        format!("{package}.{rest}")
    } else if name.contains('.') {
        name.to_owned()
    } else {
        format!("{package}.{name}")
    };
    format!("L{};", qualified.replace('.', "/"))
}

fn parse_call_arguments(text: &str, parameters: &[String]) -> Result<Vec<ArgValue>, String> {
    let parts: Vec<&str> = if text.is_empty() {
        Vec::new()
    } else {
        text.split(',').collect()
    };
    if parts.len() != parameters.len() {
        return Err(format!(
            "method butuh {} argumen, diberikan {}",
            parameters.len(),
            parts.len()
        ));
    }

    let mut values = Vec::with_capacity(parts.len());
    for (raw, parameter) in parts.iter().zip(parameters) {
        let raw = raw.trim();
        let value = match parameter.as_str() {
            "I" | "S" | "B" | "C" => ArgValue::Int(
                raw.parse::<i32>()
                    .map_err(|error| format!("bukan int: {raw} ({error})"))?,
            ),
            "Z" => ArgValue::Int(match raw {
                "true" | "1" => 1,
                "false" | "0" => 0,
                _ => return Err(format!("bukan boolean: {raw}")),
            }),
            "J" => ArgValue::Long(
                raw.parse::<i64>()
                    .map_err(|error| format!("bukan long: {raw} ({error})"))?,
            ),
            "F" => ArgValue::Float(
                raw.parse::<f32>()
                    .map_err(|error| format!("bukan float: {raw} ({error})"))?,
            ),
            "D" => ArgValue::Double(
                raw.parse::<f64>()
                    .map_err(|error| format!("bukan double: {raw} ({error})"))?,
            ),
            "Ljava/lang/String;" => ArgValue::Text(raw.trim_matches('"').to_owned()),
            other => return Err(format!("tipe parameter {other} belum didukung")),
        };
        values.push(value);
    }
    Ok(values)
}

/// Baca semua DEX yang mungkin memuat method sasaran.
///
/// Berkas `.dex` tunggal dipakai langsung; APK/XAPK/APKM dibaca per entri
/// `classes*.dex` tanpa menulis berkas sementara.
fn load_dex_candidates(path: &Path) -> CliResult<Vec<(String, Vec<u8>)>> {
    if path_looks_like_dex(path) {
        let bytes = std::fs::read(path).map_err(|error| {
            (
                ExitCode::FAILURE,
                format!("Gagal membaca '{}': {error}", path.display()),
            )
        })?;
        return Ok(vec![(path.display().to_string(), bytes)]);
    }

    let report = inspect_apk(path).map_err(|error| {
        (
            ExitCode::FAILURE,
            format!("Gagal membaca APK '{}': {error}", path.display()),
        )
    })?;

    let mut out = Vec::with_capacity(report.dex_entries.len());
    for entry in &report.dex_entries {
        let bytes = arunika_apk_loader::dex_read_from_apk(path, &entry.name).map_err(|error| {
            (
                ExitCode::FAILURE,
                format!("Gagal membaca DEX '{}': {error}", entry.name),
            )
        })?;
        out.push((entry.name.clone(), bytes));
    }
    Ok(out)
}

fn value_json(engine: &DexEngine, value: Option<Value>, return_type: &str) -> serde_json::Value {
    let Some(value) = value else {
        return serde_json::Value::Null;
    };
    match return_type {
        "V" => serde_json::Value::Null,
        "J" => serde_json::json!({"kind": "long", "value": value.as_long().unwrap_or(0)}),
        "D" => serde_json::json!({"kind": "double", "value": value.as_double().unwrap_or(0.0)}),
        "F" => serde_json::json!({"kind": "float", "value": value.as_float().unwrap_or(0.0)}),
        "I" | "S" | "B" | "C" | "Z" => {
            serde_json::json!({"kind": "int", "value": value.as_int().unwrap_or(0)})
        }
        _ => match value.as_ref().and_then(|index| engine.heap.get(index)) {
            Some(arunika_dex_engine::Object::String(text)) => {
                serde_json::json!({"kind": "string", "value": text})
            }
            Some(arunika_dex_engine::Object::Instance { type_name, .. }) => {
                serde_json::json!({"kind": "object", "type": type_name})
            }
            Some(arunika_dex_engine::Object::Array {
                element_type,
                elements,
            }) => serde_json::json!({
                "kind": "array",
                "element_type": element_type,
                "length": elements.len(),
            }),
            Some(arunika_dex_engine::Object::Class { descriptor }) => {
                serde_json::json!({"kind": "class", "descriptor": descriptor})
            }
            Some(arunika_dex_engine::Object::List {
                type_name,
                elements,
            }) => {
                serde_json::json!({"kind": "list", "type": type_name, "size": elements.len()})
            }
            Some(arunika_dex_engine::Object::Map { type_name, entries }) => {
                serde_json::json!({"kind": "map", "type": type_name, "size": entries.len()})
            }
            Some(arunika_dex_engine::Object::Method {
                class,
                name,
                parameters,
            }) => serde_json::json!({
                "kind": "method",
                "class": class,
                "name": name,
                "parameters": parameters,
            }),
            Some(arunika_dex_engine::Object::Stream {
                type_name,
                name,
                data,
                ..
            }) => serde_json::json!({
                "kind": "stream",
                "type": type_name,
                "name": name,
                "length": data.len(),
            }),
            Some(arunika_dex_engine::Object::Boxed { type_name, value }) => {
                let inner = match value {
                    arunika_dex_engine::Value::Int(number) => serde_json::json!(number),
                    arunika_dex_engine::Value::Wide(bits) => serde_json::json!(*bits as i64),
                    arunika_dex_engine::Value::Ref(_) => serde_json::Value::Null,
                };
                serde_json::json!({"kind": "boxed", "type": type_name, "value": inner})
            }
            Some(arunika_dex_engine::Object::Bitmap { width, height, .. }) => {
                serde_json::json!({"kind": "bitmap", "width": width, "height": height})
            }
            None => serde_json::json!({"kind": "ref", "value": value.as_ref()}),
        },
    }
}

fn format_value(value: &serde_json::Value) -> String {
    if value.is_null() {
        return "(tidak ada)".to_owned();
    }
    let field = |name: &str| value.get(name).cloned().unwrap_or_default();
    match value
        .get("kind")
        .and_then(|kind| kind.as_str())
        .unwrap_or("?")
    {
        "long" => format!("long {}", field("value")),
        "double" => format!("double {}", field("value")),
        "float" => format!("float {}", field("value")),
        "int" => format!("int {}", field("value")),
        "string" => format!("String {:?}", field("value")),
        "object" => format!("objek {}", field("type")),
        "array" => format!("array {}[{}]", field("element_type"), field("length")),
        _ => value.to_string(),
    }
}

fn run_method(args: impl Iterator<Item = OsString>) -> CliResult<()> {
    let request = parse_run_args(args)?;
    match (request.class.clone(), request.method.clone()) {
        (Some(class), Some(method)) => run_explicit(&request, &class, &method),
        (None, None) => run_auto(&request),
        _ => Err(bad_usage(
            "Opsi --class dan --method harus diberikan bersama.",
        )),
    }
}

/// Jalankan satu method statis secara eksplisit.
fn run_explicit(request: &RunRequest, class: &str, method: &str) -> CliResult<()> {
    let path = Path::new(&request.path);

    let mut chosen: Option<(String, DexFile, u32)> = None;
    for (name, bytes) in load_dex_candidates(path)? {
        let dex = arunika_dex_engine::parse_bytes(&bytes, &name).map_err(|error| {
            (
                ExitCode::FAILURE,
                format!("Gagal mem-parsing DEX '{name}': {error}"),
            )
        })?;
        if let Some(index) = dex.method_index(class, method)
            && dex.code_item_for_method(index as u32).is_some()
        {
            chosen = Some((name, dex, index as u32));
            break;
        }
    }

    let Some((dex_name, dex, index)) = chosen else {
        return Err((
            ExitCode::FAILURE,
            format!(
                "Method {class}->{method} tidak ditemukan atau tidak memiliki kode di '{}'.",
                request.path
            ),
        ));
    };

    let return_type = dex
        .proto_ids
        .get(dex.method_ids[index as usize].proto_idx as usize)
        .map(|proto| proto.return_type.clone())
        .unwrap_or_else(|| "V".to_owned());
    let parameters: Vec<String> = dex
        .method_parameter_types(index)
        .map(|types| types.to_vec())
        .unwrap_or_default();
    let arguments = parse_call_arguments(request.args.as_deref().unwrap_or(""), &parameters)
        .map_err(|message| bad_usage(&message))?;

    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(arunika_framework_api::AndroidNativeMethods::new()));
    engine.execute_class_initializer(class).map_err(|error| {
        (
            ExitCode::FAILURE,
            format!("Initializer {class} gagal: {error}"),
        )
    })?;

    let values: Vec<Value> = arguments
        .into_iter()
        .map(|argument| argument.materialize(&mut engine.heap))
        .collect();
    let outcome = engine
        .execute_method_values(class, method, &values)
        .map_err(|error| (ExitCode::FAILURE, format!("Eksekusi gagal: {error}")))?;

    let value = value_json(&engine, outcome.return_value, &return_type);

    if request.json {
        let payload = serde_json::json!({
            "schema_version": RUN_SCHEMA_VERSION,
            "mode": "method",
            "class": class,
            "method": method,
            "dex": dex_name,
            "executed": outcome.executed,
            "steps": outcome.steps,
            "return_type": return_type,
            "return_value": value,
        });
        let serialized = serde_json::to_string_pretty(&payload).map_err(json_error)?;
        println!("{serialized}");
    } else {
        println!("Kelas:        {class}");
        println!("Method:       {method}");
        println!("DEX:          {dex_name}");
        println!("Langkah:      {}", outcome.steps);
        println!("Tipe kembali: {return_type}");
        println!("Nilai:        {}", format_value(&value));
    }

    Ok(())
}

/// Catat eksepsi Java yang tidak tertangkap ke pelapor crash, bila terjadi.
///
/// Eksepsi yang tertangkap oleh try/catch aplikasi bukan crash; hanya
/// `UncaughtException` yang merambat keluar ke runtime yang dilaporkan.
fn report_uncaught(resolver: &arunika_framework_api::AndroidNativeMethods, error: &DexError) {
    if let DexError::UncaughtException { type_name } = error {
        resolver.record_crash(CrashType::UncaughtException, type_name.clone());
    }
}

/// Jalankan APK tanpa opsi tambahan: siapkan launcher activity.
fn run_auto(request: &RunRequest) -> CliResult<()> {
    let path = Path::new(&request.path);
    if path_looks_like_dex(path) {
        return Err(bad_usage(
            "Berkas DEX memerlukan --class dan --method; mode otomatis hanya untuk APK.",
        ));
    }

    let report = inspect_apk(path).map_err(|error| {
        (
            ExitCode::FAILURE,
            format!("Gagal membaca APK '{}': {error}", path.display()),
        )
    })?;

    let Some(activity) = report.launchable_activities.first().cloned() else {
        return Err((
            ExitCode::FAILURE,
            format!(
                "'{}' tidak memiliki activity yang dapat diluncurkan (MAIN/LAUNCHER).",
                report.package_name
            ),
        ));
    };
    let descriptor = activity_descriptor(&activity, &report.package_name);

    let mut chosen: Option<(String, DexFile)> = None;
    for (name, bytes) in load_dex_candidates(path)? {
        let dex = arunika_dex_engine::parse_bytes(&bytes, &name).map_err(|error| {
            (
                ExitCode::FAILURE,
                format!("Gagal mem-parsing DEX '{name}': {error}"),
            )
        })?;
        if dex.type_ids.iter().any(|entry| entry == &descriptor) {
            chosen = Some((name, dex));
            break;
        }
    }

    let Some((dex_name, dex)) = chosen else {
        return Err((
            ExitCode::FAILURE,
            format!("Class activity {descriptor} tidak ditemukan di DEX mana pun."),
        ));
    };

    let lifecycle: Vec<&'static str> = LIFECYCLE_METHODS
        .iter()
        .filter(|(name, parameters)| has_method(&dex, &descriptor, name, parameters))
        .map(|(name, _)| *name)
        .collect();

    // Berkas aplikasi hanya boleh berada di bawah direktori data paket ini.
    let data_root = app_data_dir(&report.package_name);
    let file_system =
        arunika_host_linux::AppDataFileSystem::new(data_root.join("files")).map_err(|error| {
            (
                ExitCode::FAILURE,
                format!("Gagal menyiapkan direktori data aplikasi: {error}"),
            )
        })?;

    // Aset dibaca langsung dari arsip APK. Kegagalan menyiapkan sumber aset
    // dilaporkan sebagai peringatan: aplikasi yang tidak memakai aset tetap
    // dapat dijalankan, sedangkan AssetManager.open akan menjelaskan
    // keterbatasannya bila dipanggil.
    let mut resolver = arunika_framework_api::AndroidNativeMethods::with_app_dir(data_root)
        .with_manifest(manifest_metadata(&report))
        .with_file_system(std::sync::Arc::new(file_system));
    match arunika_host_linux::ApkAssetSource::new(path) {
        Ok(source) => resolver = resolver.with_assets(std::sync::Arc::new(source)),
        Err(error) => eprintln!("Peringatan: aset APK tidak tersedia ({error})."),
    }
    // Resource mentah (res/) dibaca langsung dari arsip APK, mirip aset.
    match arunika_host_linux::ApkAssetSource::with_prefix(path, "res/") {
        Ok(source) => resolver = resolver.with_resources(std::sync::Arc::new(source)),
        Err(error) => eprintln!("Peringatan: resource APK tidak tersedia ({error})."),
    }
    // Simpan Arc agar keadaan jendela offscreen dapat dibaca setelah
    // eksekusi untuk pelaporan; engine mendapatkan klon yang berbagi state.
    let resolver = std::sync::Arc::new(resolver);
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(resolver.as_ref().clone()));

    // Initializer class dijalankan otomatis saat class dipakai (lihat
    // ensure_class_initialized), termasuk sebelum instance Activity dibuat.
    let mut steps = 0u64;

    let activity_object = engine.instantiate_class(&descriptor).map_err(|error| {
        (
            ExitCode::FAILURE,
            format!("Activity {activity} gagal dibuat: {error}"),
        )
    })?;
    steps += engine.last_steps;

    // Lifecycle minimal, berurutan, hanya untuk method yang ada di DEX.
    // `null` Bundle diwakili `Value::Int(0)` seperti `const/4 v0, 0`.
    let mut lifecycle_run: Vec<&str> = Vec::new();
    // Loop utama yang idle menghentikan lifecycle berikutnya: aplikasi sudah
    // mencapai titik di mana ia menunggu tugas, dan runtime tidak punya input
    // untuk memberinya pekerjaan baru.
    let mut main_loop_idle = false;
    for name in &lifecycle {
        let arguments = if *name == "onCreate" {
            vec![activity_object, Value::Int(0)]
        } else {
            vec![activity_object]
        };
        match engine.execute_method_values(&descriptor, name, &arguments) {
            Ok(_) => {}
            Err(DexError::MainLoopIdle) => {
                main_loop_idle = true;
                steps += engine.last_steps;
                break;
            }
            Err(error) => {
                report_uncaught(resolver.as_ref(), &error);
                return Err((
                    ExitCode::FAILURE,
                    format!("Lifecycle {name} pada {activity} gagal: {error}"),
                ));
            }
        }
        steps += engine.last_steps;
        lifecycle_run.push(name);
    }

    // Timer kooperatif: tugas yang dijadwalkan selama lifecycle dijalankan
    // sekali di sini, karena runtime belum memiliki jam dinding atau thread
    // penjadwal.
    // Tiga kelompok tugas kooperatif dijalankan berurutan setelah lifecycle:
    // Timer, Handler, lalu Executor. Runtime belum punya jam dinding, message
    // loop, atau thread pekerja, jadi setiap tugas dijalankan sekali di sini.
    // Batas langkah bukan kegagalan: kelompok berikutnya dilewati agar waktu
    // tidak terbuang pada loop yang sama.
    let mut counts = [0u64; 3];
    let mut step_limit = false;
    let mut loop_location: Option<String> = None;
    let stages: [CooperativeStage; 3] = [
        ("Tugas Timer", DexEngine::run_due_timers),
        ("Handler", DexEngine::run_due_handlers),
        ("Executor", DexEngine::run_due_executors),
    ];
    for (index, (label, drain)) in stages.into_iter().enumerate() {
        if step_limit || main_loop_idle {
            break;
        }
        match run_cooperative(&mut engine, &mut steps, drain) {
            Ok(run) => {
                counts[index] = run.executed;
                step_limit = run.loop_location.is_some();
                main_loop_idle = run.idle;
                loop_location = run.loop_location;
            }
            Err(error) => {
                report_uncaught(resolver.as_ref(), &error);
                return Err((
                    ExitCode::FAILURE,
                    format!("{label} pada {activity} gagal: {error}"),
                ));
            }
        }
    }
    let [timers_run, handlers_run, executors_run] = counts;

    // Masukan keyboard terprogram: event diteruskan ke activity lewat
    // dispatchKeyEvent/onKeyDown. Ini satu-satunya sumber input karena runtime
    // belum memiliki jendela host.
    let mut key_events = 0u64;
    let mut key_handled = false;
    for &code in &request.keys {
        let handled =
            dispatch_key_event(&mut engine, &descriptor, activity_object, code, &resolver)?;
        key_events += 1;
        key_handled |= handled;
    }

    // Inisialisasi otomatis bisa terjadi pada class mana pun selama lifecycle,
    // jadi status dilaporkan dari state engine, bukan dari pemanggilan eksplisit.
    let initialized = engine.class_state.initialized.contains(&descriptor);

    // Keadaan jendela offscreen diamati dari state resolver yang berbagi
    // Arc dengan engine.
    let window = resolver.window_state();

    if request.json {
        let payload = serde_json::json!({
            "schema_version": RUN_SCHEMA_VERSION,
            "mode": "activity",
            "package": report.package_name,
            "activity": activity,
            "class": descriptor,
            "dex": dex_name,
            "class_initialized": initialized,
            "steps": steps,
            "instantiated": true,
            "lifecycle": lifecycle_run,
            "timers_executed": timers_run,
            "handlers_executed": handlers_run,
            "executors_executed": executors_run,
            "main_loop_detected": step_limit,
            "main_loop_idle": main_loop_idle,
            "loop_location": loop_location,
            "window": {
                "present": window.present,
                "width": window.width,
                "height": window.height,
                "draw_calls": window.draw_calls,
            },
            "key_events": key_events,
            "key_handled": key_handled,
            "crash_reports": resolver.crash_reports().len(),
        });
        let serialized = serde_json::to_string_pretty(&payload).map_err(json_error)?;
        println!("{serialized}");
    } else {
        println!("Paket:        {}", report.package_name);
        println!("Aktivitas:    {activity}");
        println!("Class:        {descriptor}");
        println!("DEX:          {dex_name}");
        println!("Inisialisasi: {}", if initialized { "ya" } else { "belum" });
        println!("Langkah:      {steps}");
        println!(
            "Lifecycle:    {}",
            if lifecycle_run.is_empty() {
                "tidak ada".to_string()
            } else {
                lifecycle_run.join(", ")
            }
        );
        println!("Timer:        {timers_run} tugas dijalankan");
        println!("Handler:      {handlers_run} Runnable dijalankan");
        println!("Executor:     {executors_run} tugas dijalankan");
        println!(
            "Tombol:       {} event, {}ditangani",
            key_events,
            if key_handled { "" } else { "tidak " }
        );
        println!(
            "Jendela:       {}",
            if window.present {
                format!(
                    "ada ({}x{}, {} gambar)",
                    window.width, window.height, window.draw_calls
                )
            } else {
                "tidak dipakai".to_string()
            }
        );
        println!(
            "Loop utama:   {}",
            if main_loop_idle {
                "idle (tidak ada tugas tertunda)"
            } else if step_limit {
                "terdeteksi (berhenti di batas langkah)"
            } else {
                "tidak terdeteksi"
            }
        );
        if let Some(location) = &loop_location {
            println!("Lokasi loop:  {location}");
        }
        println!("Status:       Activity dibuat");
    }

    if let Some(snapshot_path) = &request.snapshot {
        let bitmaps = resolver.graphics_bitmaps();
        let Some(bitmap) = bitmaps.first() else {
            return Err((
                ExitCode::FAILURE,
                "Tidak ada bitmap offscreen untuk snapshot.".to_string(),
            ));
        };
        let mut window =
            arunika_host_linux::HostWindow::new("Arunika", bitmap.lebar, bitmap.tinggi);
        window.show();
        window.present(bitmap).map_err(|error| {
            (
                ExitCode::FAILURE,
                format!("Gagal mempresentasikan snapshot: {error}"),
            )
        })?;
        window.write_snapshot(snapshot_path).map_err(|error| {
            (
                ExitCode::FAILURE,
                format!("Gagal menulis snapshot '{snapshot_path}': {error}"),
            )
        })?;
    }

    if request.present {
        let bitmaps = resolver.graphics_bitmaps();
        let Some(bitmap) = bitmaps.first() else {
            return Err((
                ExitCode::FAILURE,
                "Tidak ada bitmap offscreen untuk presentasi host.".to_string(),
            ));
        };
        let mut window =
            arunika_host_linux::MinifbWindow::new("Arunika", bitmap.lebar, bitmap.tinggi).map_err(
                |error| {
                    (
                        ExitCode::FAILURE,
                        format!("Gagal membuka jendela host: {error}"),
                    )
                },
            )?;
        window.present(bitmap).map_err(|error| {
            (
                ExitCode::FAILURE,
                format!("Gagal mempresentasikan frame host: {error}"),
            )
        })?;
        let start = std::time::Instant::now();
        while window.is_open() && start.elapsed().as_millis() < request.present_ms as u128 {
            window.present(bitmap).ok();
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    }

    Ok(())
}

/// Teruskan satu event keyboard ke activity.
///
/// Runtime ini belum punya jendela host, sehingga input datang dari opsi
/// `--key` yang dimasukkan secara terprogram. Bila activity mengganti
/// `dispatchKeyEvent`, itu yang dipanggil; bila tidak, rute default memanggil
/// `onKeyDown` bila ada. Mengembalikan apakah event ditangani (return true).
fn dispatch_key_event(
    engine: &mut DexEngine,
    descriptor: &str,
    activity: Value,
    key_code: i32,
    resolver: &arunika_framework_api::AndroidNativeMethods,
) -> Result<bool, (ExitCode, String)> {
    const ACTION_DOWN: i32 = 0;
    const ACTION_UP: i32 = 1;
    let dispatches_key_event = has_method(
        &engine.dex,
        descriptor,
        "dispatchKeyEvent",
        &["Landroid/view/KeyEvent;"],
    );
    let handles_key_down = has_method(
        &engine.dex,
        descriptor,
        "onKeyDown",
        &["I", "Landroid/view/KeyEvent;"],
    );
    let handles_key_up = has_method(
        &engine.dex,
        descriptor,
        "onKeyUp",
        &["I", "Landroid/view/KeyEvent;"],
    );

    let mut handled = false;

    // ACTION_DOWN
    let down_event = resolver.new_key_event(&mut engine.heap, ACTION_DOWN, key_code);
    if dispatches_key_event {
        match engine.execute_method_values(descriptor, "dispatchKeyEvent", &[activity, down_event])
        {
            Ok(outcome) => handled |= matches!(outcome.return_value, Some(Value::Int(1))),
            Err(DexError::MainLoopIdle) => return Ok(false),
            Err(error) => {
                report_uncaught(resolver, &error);
                return Err((
                    ExitCode::FAILURE,
                    format!("dispatchKeyEvent pada {descriptor} gagal: {error}"),
                ));
            }
        }
    } else if handles_key_down {
        match engine.execute_method_values(
            descriptor,
            "onKeyDown",
            &[activity, Value::Int(key_code), down_event],
        ) {
            Ok(outcome) => handled |= matches!(outcome.return_value, Some(Value::Int(1))),
            Err(DexError::MainLoopIdle) => return Ok(false),
            Err(error) => {
                report_uncaught(resolver, &error);
                return Err((
                    ExitCode::FAILURE,
                    format!("onKeyDown pada {descriptor} gagal: {error}"),
                ));
            }
        }
    }

    // ACTION_UP
    let up_event = resolver.new_key_event(&mut engine.heap, ACTION_UP, key_code);
    if dispatches_key_event {
        match engine.execute_method_values(descriptor, "dispatchKeyEvent", &[activity, up_event]) {
            Ok(outcome) => handled |= matches!(outcome.return_value, Some(Value::Int(1))),
            Err(DexError::MainLoopIdle) => return Ok(false),
            Err(error) => {
                report_uncaught(resolver, &error);
                return Err((
                    ExitCode::FAILURE,
                    format!("dispatchKeyEvent pada {descriptor} gagal: {error}"),
                ));
            }
        }
    } else if handles_key_up {
        match engine.execute_method_values(
            descriptor,
            "onKeyUp",
            &[activity, Value::Int(key_code), up_event],
        ) {
            Ok(outcome) => handled |= matches!(outcome.return_value, Some(Value::Int(1))),
            Err(DexError::MainLoopIdle) => return Ok(false),
            Err(error) => {
                report_uncaught(resolver, &error);
                return Err((
                    ExitCode::FAILURE,
                    format!("onKeyUp pada {descriptor} gagal: {error}"),
                ));
            }
        }
    }

    Ok(handled)
}

/// Satu tahap tugas kooperatif: label untuk pesan galat dan fungsi drain-nya.
type CooperativeStage = (&'static str, fn(&mut DexEngine) -> Result<u64, DexError>);

/// Hasil satu tahap tugas kooperatif.
struct CooperativeRun {
    executed: u64,
    /// Method tempat batas langkah tercapai, bila itu yang terjadi.
    loop_location: Option<String>,
    /// Loop utama aplikasi tidak punya tugas tertunda lagi.
    idle: bool,
}

/// Jalankan satu kelompok tugas kooperatif dan jumlahkan langkahnya.
///
/// Batas langkah dan loop idle dikembalikan sebagai status (bukan galat)
/// karena keduanya adalah akhir wajar dari runtime satu thread; kegagalan
/// lain tetap diteruskan.
fn run_cooperative(
    engine: &mut DexEngine,
    steps: &mut u64,
    drain: fn(&mut DexEngine) -> Result<u64, DexError>,
) -> Result<CooperativeRun, DexError> {
    let result = drain(engine);
    *steps += engine.last_steps;
    match result {
        Ok(count) => Ok(CooperativeRun {
            executed: count,
            loop_location: None,
            idle: false,
        }),
        Err(DexError::ExecutionLimit { location, .. }) => Ok(CooperativeRun {
            executed: 0,
            loop_location: location,
            idle: false,
        }),
        Err(DexError::MainLoopIdle) => Ok(CooperativeRun {
            executed: 0,
            loop_location: None,
            idle: true,
        }),
        Err(error) => Err(error),
    }
}

fn load_report(path: &OsString) -> Result<ApkReport, (ExitCode, String)> {
    inspect_apk(Path::new(path)).map_err(|error| {
        (
            ExitCode::FAILURE,
            format!("Gagal membaca APK '{}': {error}", Path::new(path).display()),
        )
    })
}

/// Inspeksi DEX pertama di dalam APK tanpa menulis berkas sementara.
///
/// Kegagalan membaca atau mem-parsing DEX dikembalikan sebagai pesan, bukan
/// sebagai kegagalan perintah: metadata APK tetap berguna dan keterbatasan
/// dilaporkan secara eksplisit kepada pemanggil.
fn inspect_apk_dex(path: &OsString, report: &ApkReport) -> Result<Option<DexInspection>, String> {
    let Some(first_dex) = report.dex_entries.first() else {
        return Ok(None);
    };

    let dex_bytes = arunika_apk_loader::dex_read_from_apk(Path::new(path), &first_dex.name)
        .map_err(|e| format!("gagal membaca DEX '{}' dari APK: {e}", first_dex.name))?;

    arunika_dex_engine::inspect_dex_bytes(&dex_bytes, "info", &first_dex.name)
        .map(Some)
        .map_err(|e| format!("gagal menginspeksi DEX '{}': {e}", first_dex.name))
}

fn inspect_dex(path: &OsString, json: bool) -> Result<(), (ExitCode, String)> {
    let report = arunika_dex_engine::inspect_dex(Path::new(path), "info").map_err(|error| {
        (
            ExitCode::FAILURE,
            format!("Gagal membaca DEX '{}': {error}", Path::new(path).display()),
        )
    })?;
    if json {
        let mut payload = serde_json::to_value(&report).map_err(json_error)?;
        if let Some(object) = payload.as_object_mut() {
            object.insert("schema_version".to_owned(), INFO_SCHEMA_VERSION.into());
        }
        let serialized = serde_json::to_string_pretty(&payload).map_err(json_error)?;
        println!("{serialized}");
        Ok(())
    } else {
        print_dal_session(&report);
        Ok(())
    }
}

fn print_dal_session(report: &DexInspection) {
    println!("Berkas DEX: {}", report.path);
    println!("Versi DEX: {}", report.version);
    println!("Ukuran berkas: {} byte", report.file_size);
    println!("Ukuran header: {} byte", report.header_size);
    println!("String IDs: {}", report.string_ids_count);
    println!("Type IDs: {}", report.type_ids_count);
    println!("Proto IDs: {}", report.proto_ids_count);
    println!("Field IDs: {}", report.field_ids_count);
    println!("Method IDs: {}", report.method_ids_count);
    println!("Class defs: {}", report.class_defs_count);
    println!("Kode item: {}", report.code_items_count);
    println!("Total kata instruksi: {}", report.total_instruction_words);
}

#[cfg(test)]
mod tests {
    use super::execute;
    use std::ffi::OsString;
    use std::process::ExitCode;

    #[test]
    fn activity_descriptor_menangani_nama_relatif_dan_absolut() {
        use super::activity_descriptor;

        assert_eq!(
            activity_descriptor(".MainActivity", "com.contoh"),
            "Lcom/contoh/MainActivity;"
        );
        assert_eq!(
            activity_descriptor("com.lain.Launcher", "com.contoh"),
            "Lcom/lain/Launcher;"
        );
        assert_eq!(
            activity_descriptor("MainActivity", "com.contoh"),
            "Lcom/contoh/MainActivity;"
        );
        assert_eq!(
            activity_descriptor(".Outer$Inner", "com.contoh"),
            "Lcom/contoh/Outer$Inner;"
        );
    }

    #[test]
    fn accepts_json_option_for_info() {
        let args = ["info", "--json", "missing.apk"]
            .into_iter()
            .map(OsString::from);
        let result = execute(args);

        assert!(matches!(result, Err((code, _)) if code == ExitCode::FAILURE));
    }

    #[test]
    fn rejects_unknown_options_before_reading_file() {
        let args = ["doctor", "--future-option", "missing.apk"]
            .into_iter()
            .map(OsString::from);
        let result = execute(args);

        assert!(matches!(result, Err((code, _)) if code == ExitCode::from(2)));
    }

    #[test]
    fn info_json_dex_invalid_returns_failure() {
        let tmp = std::env::temp_dir().join("arunika-cli-fake.dex");
        std::fs::write(&tmp, b"bukan dex").expect("tidak dapat menulis file uji");
        let tmp_path = tmp.to_string_lossy().into_owned();
        let args = ["info", "--json", tmp_path.as_ref()]
            .into_iter()
            .map(OsString::from);
        let result = execute(args);
        std::fs::remove_file(&tmp).ok();
        assert!(matches!(result, Err((code, _)) if code == ExitCode::FAILURE));
    }

    // Tes ini mengetes keluaran arunika_cli, bukan DEX secara langsung.
    // DexInspection adalah bagian dari keluaran arunika_cli, jadi kita tidak boleh
    // mengetesnya di sini.
    // Jika keluaran arunika_cli tidak sesuai, perbaiki arunika_cli, bukan tes ini.
    #[test]
    fn info_json_dex_valid_contains_cmd_and_header_size() {
        let tmp = std::env::temp_dir().join(format!("arunika-cli-good-{}.dex", std::process::id()));
        let mut buf = vec![0u8; 112];
        buf[..8].copy_from_slice(b"dex\n035\0");
        buf[32..36].copy_from_slice(&(112u32).to_le_bytes());
        buf[36..40].copy_from_slice(&(112u32).to_le_bytes());
        buf[40..44].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        std::fs::write(&tmp, &buf).expect("tidak dapat menulis file uji");
        let inspection = arunika_dex_engine::inspect_dex(&tmp, "info").expect("should inspect dex");
        std::fs::remove_file(&tmp).ok();
        assert_eq!(inspection.cmd, "info");
        assert_eq!(inspection.version, "035");
        assert!(inspection.file_size >= 112);
    }
}
