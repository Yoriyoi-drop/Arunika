//! Subset API framework yang disediakan runtime (bukan bytecode DEX).
//!
//! Fase 1 mencakup fungsi murni (`Math`) dan operasi `String`/`Integer` yang
//! butuh akses heap. Method yang tidak ada di sini dilaporkan sebagai tidak
//! tersedia oleh interpreter, bukan diam-diam mengembalikan nilai palsu.

use std::sync::Arc;

use arunika_audio_manager::AudioManager;
use arunika_crash_reporter::{CrashReporter, CrashType};
use arunika_dex_engine::{Heap, NativeException, NativeResolver, Object, Value};
use arunika_graphics_engine::GraphicsEngine;
use arunika_native_bridge::{JavaVm, JniError, JniValue};
use arunika_sandbox_manager::{AssetSource, ConfinedFileSystem, PermissionPolicy};
use arunika_service_broker::ClipboardManager;
use arunika_threading::ThreadManager;
use arunika_view_engine::{LayoutParams, View, ViewHierarchy, ViewKind};
use num_bigint::{BigInt, Sign};
use num_traits::{Signed, ToPrimitive};
use sha1::Digest as _;

/// Ukuran layar virtual yang dilaporkan ke aplikasi.
///
/// Runtime belum membuka jendela atau permukaan gambar, jadi angka ini tetap
/// sampai fase window/render tersedia.
const VIRTUAL_WIDTH: i32 = 1920;
const VIRTUAL_HEIGHT: i32 = 1080;
const VIRTUAL_DPI: i32 = 320;

/// MAC placeholder yang juga dilaporkan Android ke aplikasi tanpa izin.
const VIRTUAL_MAC: &str = "02:00:00:00:00:00";

/// Metadata manifest yang boleh dilaporkan `PackageManager`.
///
/// Nilainya berasal dari manifest APK yang sudah diurai pemanggil. Semua field
/// opsional: runtime tidak mengarang nilai saat manifest tidak menyediakannya.
#[derive(Debug, Default, Clone)]
pub struct AppManifest {
    pub package: Option<String>,
    pub version_name: Option<String>,
    pub version_code: Option<i64>,
    pub target_sdk: Option<i32>,
    pub min_sdk: Option<i32>,
    pub label: Option<String>,
    /// Pasangan `name`/`value` dari `<meta-data>` pada manifest.
    pub meta_data: Vec<(String, String)>,
    /// Izin yang dideklarasikan manifest. Runtime menganggap izin ini diberikan
    /// saat pemasangan; izin lain dilaporkan ditolak.
    pub permissions: Vec<String>,
}

/// Keadaan jendela offscreen yang diamati oleh perintah `run`.
///
/// Runtime ini tidak menggambar ke layar nyata; operasi kanvas merekam ke
/// framebuffer di memori. Keadaan ini dibaca setelah eksekusi agar laporan
/// dapat menjelaskan apakah aplikasi benar-benar menggunakan jendela.
#[derive(Debug, Default, Clone)]
pub struct WindowState {
    /// `true` bila aplikasi memanggil `setContentView` atau menggambar kanvas.
    pub present: bool,
    /// Lebar jendela/offscreen yang dikonfigurasi, dalam piksel.
    pub width: u32,
    /// Tinggi jendela/offscreen yang dikonfigurasi, dalam piksel.
    pub height: u32,
    /// Jumlah operasi gambar yang terekam (drawColor/drawRect/drawText/...).
    pub draw_calls: u64,
    /// Jumlah view dalam hierarki setelah tata letak terakhir.
    pub view_count: usize,
}

/// Implementasi awal API android/java yang disediakan runtime.
///
/// Semua manajer domain (jendela, grafik, audio, clipboard, thread, native
/// JNI, dan pelapor crash) dibungkus `Arc<Mutex<_>>` agar state dapat dibagi
/// antara resolver (yang diklon ke mesin DEX) dan pemanggil CLI setelah
/// eksekusi selesai, tanpa melanggar aturan dependensi satu arah.
#[derive(Debug, Clone)]
pub struct AndroidNativeMethods {
    /// Direktori data aplikasi virtual untuk `getFilesDir` dan sejenisnya.
    /// Runtime belum melakukan I/O disk; path hanya dipakai sebagai nilai
    /// yang dikembalikan ke kode aplikasi.
    app_dir: Option<std::path::PathBuf>,
    /// Metadata manifest untuk `Context.getPackageName()` dan `PackageManager`.
    manifest: AppManifest,
    /// Kebijakan izin yang diturunkan dari manifest.
    permission_policy: PermissionPolicy,
    /// Objek `SharedPreferences` per nama agar isinya bertahan dalam proses.
    prefs: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Value>>>,
    /// Berkas aplikasi yang boleh diakses runtime. Tanpa ini, API berkas
    /// dilaporkan gagal alih-alih membaca disk secara diam-diam.
    file_system: Option<Arc<dyn ConfinedFileSystem>>,
    /// Aset read-only dari dalam APK. Tanpa ini, `AssetManager` melaporkan
    /// bahwa runtime tidak diberi akses aset alih-alih mengembalikan aliran
    /// kosong yang menyesatkan.
    assets: Option<Arc<dyn AssetSource>>,
    /// Resource mentah dari dalam APK (`res/`). Tanpa ini, `Resources`
    /// melaporkan bahwa tabel resource belum diurai.
    resources: Option<Arc<dyn AssetSource>>,
    /// Keadaan jendela offscreen yang diamati pelaporan `run`.
    window: std::sync::Arc<std::sync::Mutex<WindowState>>,
    /// Hierarki view aplikasi (layout yang diinflasi lewat setContentView).
    view_hierarchy: std::sync::Arc<std::sync::Mutex<ViewHierarchy>>,
    /// Pemetaan objek View (referensi heap) ke id node pada hierarki view.
    view_nodes: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u32, u32>>>,
    /// Id Android (`View.getId`/`setId`) per objek View, bila diset aplikasi.
    view_android_ids: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u32, i32>>>,
    /// Engine grafis offscreen yang menampung Bitmap hasil operasi kanvas.
    graphics_engine: std::sync::Arc<std::sync::Mutex<GraphicsEngine>>,
    /// Manajer audio dalam proses (model, tanpa backend host nyata).
    audio_manager: std::sync::Arc<std::sync::Mutex<AudioManager>>,
    /// Manajer clipboard dalam proses (model, tanpa backend host nyata).
    clipboard_manager: std::sync::Arc<std::sync::Mutex<ClipboardManager>>,
    /// Manajer thread dalam proses (model, tanpa thread OS nyata).
    thread_manager: std::sync::Arc<std::sync::Mutex<ThreadManager>>,
    /// Mesin virtual Java model untuk panggilan JNI.
    java_vm: std::sync::Arc<std::sync::Mutex<JavaVm>>,
    /// Pelapor crash untuk menangkap eksepsi tidak tertangkap selama eksekusi.
    crash_reporter: std::sync::Arc<std::sync::Mutex<CrashReporter>>,
}

impl Default for AndroidNativeMethods {
    fn default() -> Self {
        Self {
            app_dir: None,
            manifest: AppManifest::default(),
            permission_policy: PermissionPolicy::default(),
            prefs: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            file_system: None,
            assets: None,
            resources: None,
            window: std::sync::Arc::new(std::sync::Mutex::new(WindowState::default())),
            view_hierarchy: std::sync::Arc::new(std::sync::Mutex::new(ViewHierarchy::new())),
            view_nodes: std::sync::Arc::new(
                std::sync::Mutex::new(std::collections::HashMap::new()),
            ),
            view_android_ids: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            graphics_engine: std::sync::Arc::new(std::sync::Mutex::new(GraphicsEngine::baru())),
            audio_manager: std::sync::Arc::new(std::sync::Mutex::new(AudioManager::new())),
            clipboard_manager: std::sync::Arc::new(std::sync::Mutex::new(ClipboardManager::new())),
            thread_manager: std::sync::Arc::new(std::sync::Mutex::new(ThreadManager::baru())),
            java_vm: std::sync::Arc::new(std::sync::Mutex::new(JavaVm::baru())),
            crash_reporter: std::sync::Arc::new(std::sync::Mutex::new(CrashReporter::new())),
        }
    }
}

impl AndroidNativeMethods {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_app_dir(dir: std::path::PathBuf) -> Self {
        Self {
            app_dir: Some(dir),
            ..Self::default()
        }
    }

    pub fn with_package(mut self, package: impl Into<String>) -> Self {
        self.manifest.package = Some(package.into());
        self
    }

    /// Mengganti seluruh metadata manifest sekaligus.
    pub fn with_manifest(mut self, manifest: AppManifest) -> Self {
        self.permission_policy = PermissionPolicy::from_permissions(&manifest.permissions);
        self.manifest = manifest;
        self
    }

    pub fn with_permissions(mut self, permissions: Vec<String>) -> Self {
        self.manifest.permissions = permissions.clone();
        self.permission_policy = PermissionPolicy::from_permissions(&permissions);
        self
    }

    /// Izin yang dideklarasikan manifest dianggap diberikan saat pemasangan.
    fn permission_granted(&self, name: &str) -> bool {
        self.permission_policy.is_granted(name)
    }

    /// Memberi runtime akses ke direktori data aplikasi yang sudah dikonfinemen.
    pub fn with_file_system(mut self, file_system: Arc<dyn ConfinedFileSystem>) -> Self {
        self.file_system = Some(file_system);
        self
    }

    /// Memberi runtime akses baca ke aset di dalam APK.
    pub fn with_assets(mut self, assets: Arc<dyn AssetSource>) -> Self {
        self.assets = Some(assets);
        self
    }

    /// Akses ke keadaan jendela offscreen untuk pelaporan setelah eksekusi.
    pub fn window_state(&self) -> WindowState {
        self.window.lock().expect("kunci jendela").clone()
    }

    /// Hierarki view aplikasi setelah eksekusi (untuk pelaporan `run`).
    pub fn view_hierarchy(&self) -> ViewHierarchy {
        self.view_hierarchy
            .lock()
            .expect("kunci hierarki view")
            .clone()
    }

    /// Hasil render grafis offscreen setelah eksekusi (untuk pelaporan `run`).
    pub fn graphics_bitmaps(&self) -> Vec<arunika_graphics_engine::Bitmap> {
        self.graphics_engine
            .lock()
            .expect("kunci engine grafis")
            .bitmaps()
    }

    /// Manajer audio dalam proses (untuk pelaporan `run`).
    pub fn audio_manager(&self) -> AudioManager {
        self.audio_manager.lock().expect("kunci audio").clone()
    }

    /// Manajer clipboard dalam proses (untuk pelaporan `run`).
    pub fn clipboard_manager(&self) -> ClipboardManager {
        self.clipboard_manager
            .lock()
            .expect("kunci clipboard")
            .clone()
    }

    /// Manajer thread dalam proses (untuk pelaporan `run`).
    pub fn thread_manager(&self) -> ThreadManager {
        self.thread_manager.lock().expect("kunci thread").clone()
    }

    /// Mesin virtual Java model (untuk pelaporan `run`).
    pub fn java_vm(&self) -> JavaVm {
        self.java_vm.lock().expect("kunci JVM").clone()
    }

    /// Daftarkan implementasi method native pada JVM model.
    pub fn register_native_method<F>(
        &self,
        class_name: impl Into<String>,
        method_name: impl Into<String>,
        f: F,
    ) where
        F: Fn(&[JniValue]) -> Result<JniValue, JniError> + Send + Sync + 'static,
    {
        self.java_vm
            .lock()
            .expect("kunci JVM")
            .dapatkan_env()
            .daftarkan_native(class_name, method_name, f);
    }

    /// Panggil method native yang terdaftar pada JVM model.
    pub fn call_native_method(
        &self,
        class_name: &str,
        method_name: &str,
        args: &[JniValue],
    ) -> Result<JniValue, String> {
        self.java_vm
            .lock()
            .expect("kunci JVM")
            .dapatkan_env()
            .panggil_native(class_name, method_name, args)
            .map_err(|error| error.to_string())
    }

    /// Laporan crash yang tertangkap selama eksekusi (untuk pelaporan `run`).
    pub fn crash_reports(&self) -> Vec<arunika_crash_reporter::CrashReport> {
        self.crash_reporter
            .lock()
            .expect("kunci pelapor crash")
            .reports()
            .to_vec()
    }

    /// Catat satu crash yang terjadi selama eksekusi aplikasi.
    pub fn record_crash(&self, crash_type: CrashType, message: String) {
        let _ = self
            .crash_reporter
            .lock()
            .expect("kunci pelapor crash")
            .report(crash_type, message);
    }

    /// Memberi runtime akses baca ke resource mentah (`res/`) di dalam APK.
    pub fn with_resources(mut self, resources: Arc<dyn AssetSource>) -> Self {
        self.resources = Some(resources);
        self
    }

    /// Resolusi nama resource ke id sintetik berdasarkan entri `res/` yang ada.
    fn resource_entry_matches(entry: &str, def_type: Option<&str>, name: &str) -> bool {
        let mut parts = entry.splitn(2, '/');
        let folder = parts.next().unwrap_or("");
        let file = parts.next().unwrap_or("");
        let stem = file.rsplit_once('.').map(|(left, _)| left).unwrap_or(file);
        let type_ok = match def_type {
            Some(type_name) => folder == type_name || folder.split('-').next() == Some(type_name),
            None => true,
        };
        type_ok && stem == name
    }

    fn resolve_resource(&self, name: Option<&str>, def_type: Option<&str>) -> Option<i32> {
        let resources = self.resources.as_ref()?;
        let names = resources.entries().ok()?;
        let name = name?;
        // Prefiks kualifier (mis. `layout-land`) hanya dipakai bila tidak ada
        // entri dasar dengan tipe yang persis sama, agar resource default
        // (tanpa kualifier) dipilih terlebih dahulu.
        let exact = names.iter().position(|entry| {
            let folder = entry.split('/').next().unwrap_or("");
            folder == def_type.unwrap_or(folder)
                && Self::resource_entry_matches(entry, def_type, name)
        });
        if let Some(index) = exact {
            return Some((index + 1) as i32);
        }
        names
            .iter()
            .position(|entry| Self::resource_entry_matches(entry, def_type, name))
            .map(|index| (index + 1) as i32)
    }

    /// Baca resource mentah berdasarkan id sintetik yang dihasilkan
    /// `resolve_resource`.
    fn read_resource(&self, id: i32) -> Option<Vec<u8>> {
        let resources = self.resources.as_ref()?;
        let names = resources.entries().ok()?;
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        let entry = names.get(index)?;
        resources.read(entry).ok()?
    }

    fn require_assets(&self) -> Result<&dyn AssetSource, String> {
        self.assets
            .as_deref()
            .ok_or_else(|| "runtime tidak diberi akses aset APK".to_string())
    }

    fn require_file_system(&self) -> Result<&dyn ConfinedFileSystem, String> {
        self.file_system
            .as_deref()
            .ok_or_else(|| "runtime tidak diberi akses berkas aplikasi".to_string())
    }

    fn int_args(args: &[Value]) -> Result<Vec<i32>, String> {
        args.iter()
            .map(|value| {
                value
                    .as_int()
                    .ok_or_else(|| "argumen harus bertipe int".to_string())
            })
            .collect()
    }

    fn string_arg(heap: &Heap, args: &[Value], index: usize) -> Result<String, String> {
        let value = args
            .get(index)
            .ok_or_else(|| format!("argumen ke-{} tidak ada", index + 1))?;
        let reference = value.as_ref().ok_or("argumen harus String")?;
        heap.string(reference)
            .map(|text| text.to_owned())
            .ok_or_else(|| "argumen bukan String".to_string())
    }

    fn int_arg(args: &[Value], index: usize) -> Result<i32, String> {
        args.get(index)
            .and_then(|value| value.as_int())
            .ok_or_else(|| format!("argumen ke-{} harus int", index + 1))
    }

    /// Teks sebuah `CharSequence`: `String` langsung, atau `StringBuilder`
    /// yang menyimpan teksnya pada field 0.
    fn char_sequence_text(heap: &Heap, value: Value) -> Option<String> {
        let reference = value.as_ref()?;
        match heap.get(reference)? {
            Object::String(text) => Some(text.clone()),
            Object::Instance { fields, .. } => fields
                .get(&0)
                .and_then(|value| value.as_ref())
                .and_then(|text| heap.string(text))
                .map(|text| text.to_owned()),
            _ => None,
        }
    }

    /// Representasi teks sebuah objek seperti String.valueOf.
    ///
    /// Null menjadi "null", String memakai isinya, nilai boxed memakai teks
    /// primitifnya, dan instance lain memakai bentuk identitas Java
    /// (Nama@hash) karena runtime belum menyediakan toString virtual.
    fn object_text(heap: &Heap, value: Value) -> String {
        match value {
            Value::Int(0) => "null".to_string(),
            Value::Ref(reference) => match heap.get(reference) {
                Some(Object::String(text)) => text.clone(),
                Some(Object::Boxed { type_name, value }) => Self::boxed_text(type_name, *value),
                Some(Object::Instance { type_name, fields }) => {
                    let name = Self::java_name(type_name);
                    if name.ends_with("StringBuilder") || name.ends_with("StringBuffer") {
                        fields
                            .get(&0)
                            .and_then(|value| value.as_ref())
                            .and_then(|text| heap.string(text))
                            .map(|text| text.to_owned())
                            .unwrap_or_default()
                    } else {
                        format!("{name}@{reference:x}")
                    }
                }
                Some(Object::Class { descriptor }) => {
                    format!("class {}", Self::java_name(descriptor))
                }
                Some(_) => format!("objek@{reference:x}"),
                None => "null".to_string(),
            },
            Value::Int(raw) => raw.to_string(),
            Value::Wide(raw) => (raw as i64).to_string(),
        }
    }

    /// Teks sebuah nilai boxed sesuai tipe pembungkusnya.
    fn boxed_text(type_name: &str, value: Value) -> String {
        match (type_name, value) {
            ("Ljava/lang/Boolean;", Value::Int(raw)) => (raw != 0).to_string(),
            ("Ljava/lang/Character;", Value::Int(raw)) => u32::try_from(raw)
                .ok()
                .and_then(char::from_u32)
                .map(|character| character.to_string())
                .unwrap_or_default(),
            ("Ljava/lang/Float;", Value::Int(raw)) => {
                Self::float_text_f32(f32::from_bits(raw as u32))
            }
            ("Ljava/lang/Double;", Value::Wide(raw)) => Self::float_text_f64(f64::from_bits(raw)),
            ("Ljava/lang/Long;", Value::Wide(raw)) => (raw as i64).to_string(),
            (_, Value::Int(raw)) => raw.to_string(),
            (_, Value::Wide(raw)) => (raw as i64).to_string(),
            (_, Value::Ref(_)) => "null".to_string(),
        }
    }

    /// Teks sebuah float. Java menulis 1.0 sebagai "1.0", sama seperti
    /// format Debug Rust; notasi eksponen bisa berbeda dari Java.
    fn float_text_f32(value: f32) -> String {
        if value.is_finite() {
            format!("{value:?}")
        } else if value.is_nan() {
            "NaN".to_string()
        } else if value > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        }
    }

    fn float_text_f64(value: f64) -> String {
        if value.is_finite() {
            format!("{value:?}")
        } else if value.is_nan() {
            "NaN".to_string()
        } else if value > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        }
    }
    /// Descriptor DEX dari nama kelas Java yang diberikan kode aplikasi.
    fn descriptor_from_java_name(name: &str) -> String {
        match name {
            "int" => "I".to_string(),
            "long" => "J".to_string(),
            "boolean" => "Z".to_string(),
            "byte" => "B".to_string(),
            "char" => "C".to_string(),
            "short" => "S".to_string(),
            "float" => "F".to_string(),
            "double" => "D".to_string(),
            "void" => "V".to_string(),
            _ => format!("L{};", name.replace('.', "/")),
        }
    }

    fn replace_with_string(heap: &mut Heap, reference: u32, text: String) -> Result<(), String> {
        match heap.get_mut(reference) {
            Some(slot) => {
                *slot = Object::String(text);
                Ok(())
            }
            None => Err("objek String tidak ada di heap".to_string()),
        }
    }

    /// Teks yang disimpan pada StringBuilder; field 0 berisi referensi String.
    fn builder_text(heap: &Heap, reference: u32) -> Option<String> {
        match heap.get(reference)? {
            Object::Instance { fields, .. } => fields
                .get(&0)
                .and_then(|value| value.as_ref())
                .and_then(|text| heap.string(text))
                .map(|text| text.to_owned()),
            _ => None,
        }
    }

    fn set_builder_text(heap: &mut Heap, reference: u32, text: String) -> Result<(), String> {
        let string = heap.alloc(Object::String(text));
        match heap.get_mut(reference) {
            Some(Object::Instance { fields, .. }) => {
                fields.insert(0, string);
                Ok(())
            }
            _ => Err("objek bukan instance".to_string()),
        }
    }

    fn this_ref(args: &[Value]) -> Result<u32, String> {
        args.first()
            .and_then(|value| value.as_ref())
            .ok_or_else(|| "method instance tanpa objek penerima".to_string())
    }

    /// `java.util.Arrays`: utilitas array.
    fn arrays_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("equals", "([Ljava/lang/Object;[Ljava/lang/Object;)Z") => {
                Some(Ok(Value::Int(i32::from(Self::arrays_equal(
                    heap,
                    args.first().copied().unwrap_or(Value::Int(0)),
                    args.get(1).copied().unwrap_or(Value::Int(0)),
                    false,
                )))))
            }
            ("deepEquals", "([Ljava/lang/Object;[Ljava/lang/Object;)Z") => {
                Some(Ok(Value::Int(i32::from(Self::arrays_equal(
                    heap,
                    args.first().copied().unwrap_or(Value::Int(0)),
                    args.get(1).copied().unwrap_or(Value::Int(0)),
                    true,
                )))))
            }
            ("toString", "([Ljava/lang/Object;)Ljava/lang/String;") => {
                Some(Ok(heap.alloc(Object::String(Self::array_to_string(
                    heap,
                    args.first().copied().unwrap_or(Value::Int(0)),
                )))))
            }
            ("fill", "([Ljava/lang/Object;Ljava/lang/Object;)V") => Some((|| {
                let array = args.first().copied().unwrap_or(Value::Int(0));
                let value = args.get(1).copied().unwrap_or(Value::Int(0));
                let reference = array.as_ref().ok_or("array null")?;
                match heap.get_mut(reference) {
                    Some(Object::Array { elements, .. }) => {
                        for slot in elements.iter_mut() {
                            *slot = value;
                        }
                        Ok(Value::Int(0))
                    }
                    _ => Err("bukan array".to_string()),
                }
            })()),
            ("copyOf", "([Ljava/lang/Object;I)[Ljava/lang/Object;") => Some((|| {
                let original = args.first().copied().unwrap_or(Value::Int(0));
                let new_len = usize::try_from(Self::int_arg(args, 1)?)
                    .map_err(|_| "panjang negatif".to_string())?;
                let reference = original.as_ref().ok_or("array null")?;
                let (element_type, elements) = match heap.get(reference) {
                    Some(Object::Array {
                        element_type,
                        elements,
                    }) => (element_type.clone(), elements.clone()),
                    _ => return Err("bukan array".to_string()),
                };
                let mut out = vec![Value::Int(0); new_len];
                for (i, value) in elements.into_iter().take(new_len).enumerate() {
                    out[i] = value;
                }
                Ok(heap.alloc(Object::Array {
                    element_type,
                    elements: out,
                }))
            })()),
            ("sort", "([Ljava/lang/Object;)V") => Some((|| {
                let array = args.first().copied().unwrap_or(Value::Int(0));
                let reference = array.as_ref().ok_or("array null")?;
                let snapshot = match heap.get(reference) {
                    Some(Object::Array { elements, .. }) => elements.clone(),
                    _ => return Err("bukan array".to_string()),
                };
                let keys: Vec<String> = snapshot
                    .iter()
                    .map(|v| Self::object_text(heap, *v))
                    .collect();
                let mut order: Vec<usize> = (0..snapshot.len()).collect();
                order.sort_by(|&i, &j| keys[i].cmp(&keys[j]));
                let sorted: Vec<Value> = order.into_iter().map(|i| snapshot[i]).collect();
                match heap.get_mut(reference) {
                    Some(Object::Array { elements, .. }) => {
                        elements.clone_from(&sorted);
                        Ok(Value::Int(0))
                    }
                    _ => Err("bukan array".to_string()),
                }
            })()),
            _ => None,
        }
    }

    fn arrays_equal(heap: &Heap, left: Value, right: Value, _deep: bool) -> bool {
        if left == right {
            return true;
        }
        let (Some(lref), Some(rref)) = (left.as_ref(), right.as_ref()) else {
            return false;
        };
        let (Some(Object::Array { elements: le, .. }), Some(Object::Array { elements: re, .. })) =
            (heap.get(lref), heap.get(rref))
        else {
            return false;
        };
        if le.len() != re.len() {
            return false;
        }
        le.iter()
            .zip(re)
            .all(|(a, b)| *a == *b || Self::objects_equal(heap, *a, *b))
    }

    fn array_to_string(heap: &Heap, value: Value) -> String {
        let Some(reference) = value.as_ref() else {
            return "null".to_string();
        };
        let Some(Object::Array { elements, .. }) = heap.get(reference) else {
            return Self::object_text(heap, value);
        };
        let parts: Vec<String> = elements
            .iter()
            .map(|element| Self::object_text(heap, *element))
            .collect();
        format!("[{}]", parts.join(", "))
    }

    /// `java.util.Objects`: utilitas perbandingan, hash, dan null-check.
    fn objects_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("equals", "(Ljava/lang/Object;Ljava/lang/Object;)Z") => {
                Some(Ok(Value::Int(i32::from(Self::objects_equal(
                    heap,
                    args.first().copied().unwrap_or(Value::Int(0)),
                    args.get(1).copied().unwrap_or(Value::Int(0)),
                )))))
            }
            ("isNull", "(Ljava/lang/Object;)Z") => Some(Ok(Value::Int(i32::from(
                args.first().copied().unwrap_or(Value::Int(0)) == Value::Int(0),
            )))),
            ("nonNull", "(Ljava/lang/Object;)Z") => Some(Ok(Value::Int(i32::from(
                args.first().copied().unwrap_or(Value::Int(0)) != Value::Int(0),
            )))),
            ("hashCode", "(Ljava/lang/Object;)I") => Some(Ok(Value::Int(Self::object_hash(
                heap,
                args.first().copied().unwrap_or(Value::Int(0)),
            )))),
            ("toString", "(Ljava/lang/Object;)Ljava/lang/String;")
            | ("toString", "(Ljava/lang/Object;Ljava/lang/String;)Ljava/lang/String;") => {
                Some(Ok(heap.alloc(Object::String(Self::object_text(
                    heap,
                    args.first().copied().unwrap_or(Value::Int(0)),
                )))))
            }
            ("requireNonNull", "(Ljava/lang/Object;)Ljava/lang/Object;") => Some((|| {
                let value = args.first().copied().unwrap_or(Value::Int(0));
                if value == Value::Int(0) {
                    return Err(NativeException::new(
                        "java.lang.NullPointerException",
                        "objek tidak boleh null".to_string(),
                    )
                    .encode());
                }
                Ok(value)
            })()),
            ("requireNonNull", "(Ljava/lang/Object;Ljava/lang/String;)Ljava/lang/Object;") => {
                Some((|| {
                    let value = args.first().copied().unwrap_or(Value::Int(0));
                    if value == Value::Int(0) {
                        let message = Self::string_arg(heap, args, 1).unwrap_or_default();
                        return Err(NativeException::new(
                            "java.lang.NullPointerException",
                            if message.is_empty() {
                                "objek tidak boleh null".to_string()
                            } else {
                                message
                            },
                        )
                        .encode());
                    }
                    Ok(value)
                })())
            }
            ("deepEquals", "(Ljava/lang/Object;Ljava/lang/Object;)Z") => {
                Some(Ok(Value::Int(i32::from(Self::objects_equal(
                    heap,
                    args.first().copied().unwrap_or(Value::Int(0)),
                    args.get(1).copied().unwrap_or(Value::Int(0)),
                )))))
            }
            _ => None,
        }
    }

    fn object_hash(heap: &Heap, value: Value) -> i32 {
        match value {
            Value::Int(0) => 0,
            Value::Ref(reference) => match heap.get(reference) {
                Some(Object::String(text)) => text.bytes().fold(0i32, |acc, b| {
                    acc.wrapping_mul(31).wrapping_add(i32::from(b))
                }),
                Some(_) => reference as i32,
                None => 0,
            },
            Value::Int(raw) => raw,
            Value::Wide(raw) => (raw as i64) as i32,
        }
    }

    fn objects_equal(heap: &Heap, left: Value, right: Value) -> bool {
        if left == right {
            return true;
        }
        if left == Value::Int(0) || right == Value::Int(0) {
            return false;
        }
        match (left, right) {
            (Value::Ref(a), Value::Ref(b)) => match (heap.get(a), heap.get(b)) {
                (Some(Object::String(x)), Some(Object::String(y))) => x == y,
                _ => a == b,
            },
            _ => false,
        }
    }

    fn math_method(method: &str, signature: &str, args: &[Value]) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("abs", "(I)I") => Some(Self::int_args(args).and_then(|values| {
                let value = values.first().copied().ok_or("Math.abs butuh 1 argumen")?;
                Ok(Value::Int(value.wrapping_abs()))
            })),
            ("abs", "(J)J") => Some(Self::long_args(args).and_then(|values| {
                let value = values.first().copied().ok_or("Math.abs butuh 1 argumen")?;
                Ok(Value::from_long(value.wrapping_abs()))
            })),
            ("abs", "(F)F") => Some(Self::float_args(args).and_then(|values| {
                let value = values.first().copied().ok_or("Math.abs butuh 1 argumen")?;
                Ok(Value::from_float(value.abs()))
            })),
            ("abs", "(D)D") => Some(Self::double_args(args).and_then(|values| {
                let value = values.first().copied().ok_or("Math.abs butuh 1 argumen")?;
                Ok(Value::from_double(value.abs()))
            })),
            ("max", "(II)I") => Some(Self::int_args(args).and_then(|values| {
                let [a, b] = values.as_slice() else {
                    return Err("Math.max butuh 2 argumen".to_string());
                };
                Ok(Value::Int(*a.max(b)))
            })),
            ("min", "(II)I") => Some(Self::int_args(args).and_then(|values| {
                let [a, b] = values.as_slice() else {
                    return Err("Math.min butuh 2 argumen".to_string());
                };
                Ok(Value::Int(*a.min(b)))
            })),
            ("max", "(JJ)J") => Some(Self::long_args(args).and_then(|values| {
                let [a, b] = values.as_slice() else {
                    return Err("Math.max butuh 2 argumen".to_string());
                };
                Ok(Value::from_long(*a.max(b)))
            })),
            ("min", "(JJ)J") => Some(Self::long_args(args).and_then(|values| {
                let [a, b] = values.as_slice() else {
                    return Err("Math.min butuh 2 argumen".to_string());
                };
                Ok(Value::from_long(*a.min(b)))
            })),
            ("max", "(DD)D") => Some(Self::double_args(args).and_then(|values| {
                let [a, b] = values.as_slice() else {
                    return Err("Math.max butuh 2 argumen".to_string());
                };
                Ok(Value::from_double(a.max(*b)))
            })),
            ("min", "(DD)D") => Some(Self::double_args(args).and_then(|values| {
                let [a, b] = values.as_slice() else {
                    return Err("Math.min butuh 2 argumen".to_string());
                };
                Ok(Value::from_double(a.min(*b)))
            })),
            ("sqrt", "(D)D") => Some(Self::double_args(args).and_then(|values| {
                let value = values.first().copied().ok_or("Math.sqrt butuh 1 argumen")?;
                Ok(Value::from_double(value.sqrt()))
            })),
            ("round", "(F)I") => Some(Self::float_args(args).and_then(|values| {
                let value = values
                    .first()
                    .copied()
                    .ok_or("Math.round butuh 1 argumen")?;
                Ok(Value::Int(value.round() as i32))
            })),
            ("round", "(D)J") => Some(Self::double_args(args).and_then(|values| {
                let value = values
                    .first()
                    .copied()
                    .ok_or("Math.round butuh 1 argumen")?;
                Ok(Value::from_long(value.round() as i64))
            })),
            _ => None,
        }
    }

    fn long_args(args: &[Value]) -> Result<Vec<i64>, String> {
        args.iter()
            .map(|value| {
                value
                    .as_long()
                    .ok_or_else(|| "argumen harus long".to_string())
            })
            .collect()
    }

    fn long_arg(args: &[Value], index: usize) -> Result<i64, String> {
        args.get(index)
            .and_then(|value| value.as_long())
            .ok_or_else(|| format!("argumen ke-{} harus long", index + 1))
    }

    fn float_args(args: &[Value]) -> Result<Vec<f32>, String> {
        args.iter()
            .map(|value| {
                value
                    .as_float()
                    .ok_or_else(|| "argumen harus float".to_string())
            })
            .collect()
    }

    fn double_args(args: &[Value]) -> Result<Vec<f64>, String> {
        args.iter()
            .map(|value| {
                value
                    .as_double()
                    .ok_or_else(|| "argumen harus double".to_string())
            })
            .collect()
    }

    /// Potong string berdasarkan satuan UTF-16, sesuai semantik Java.
    fn utf16_slice(text: &str, begin: usize, end: Option<usize>) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        let end = end.unwrap_or(units.len()).min(units.len());
        let begin = begin.min(end);
        String::from_utf16_lossy(&units[begin..end])
    }

    fn string_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            // Konstruktor mengganti instance kosong menjadi String di heap.
            ("<init>", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                Self::replace_with_string(heap, reference, String::new())?;
                Ok(Value::Int(0))
            })()),
            ("<init>", "(Ljava/lang/String;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let text = Self::string_arg(heap, args, 1)?;
                Self::replace_with_string(heap, reference, text)?;
                Ok(Value::Int(0))
            })()),
            ("<init>", "([B)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let bytes = Self::byte_array_arg(heap, args, 1)?;
                let text = String::from_utf8_lossy(&bytes).into_owned();
                Self::replace_with_string(heap, reference, text)?;
                Ok(Value::Int(0))
            })()),
            ("<init>", "([BII)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let bytes = Self::byte_array_arg(heap, args, 1)?;
                let offset = usize::try_from(Self::int_arg(args, 2)?)
                    .map_err(|_| "offset negatif".to_string())?;
                let length = usize::try_from(Self::int_arg(args, 3)?)
                    .map_err(|_| "panjang negatif".to_string())?;
                let end = offset.saturating_add(length);
                let slice = bytes
                    .get(offset..end)
                    .ok_or_else(|| format!("rentang {offset}..{end} melebihi {}", bytes.len()))?;
                let text = String::from_utf8_lossy(slice).into_owned();
                Self::replace_with_string(heap, reference, text)?;
                Ok(Value::Int(0))
            })()),
            ("length", "()I") => Some(
                Self::string_arg(heap, args, 0)
                    .map(|text| Value::Int(text.encode_utf16().count() as i32)),
            ),
            ("isEmpty", "()Z") => Some(
                Self::string_arg(heap, args, 0).map(|text| Value::Int(i32::from(text.is_empty()))),
            ),
            ("charAt", "(I)C") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let index = Self::int_arg(args, 1)?;
                let units: Vec<u16> = text.encode_utf16().collect();
                let unit = usize::try_from(index)
                    .ok()
                    .and_then(|slot| units.get(slot))
                    .copied()
                    .ok_or_else(|| format!("indeks {index} di luar panjang {}", units.len()))?;
                Ok(Value::Int(i32::from(unit)))
            })()),
            ("equals", "(Ljava/lang/Object;)Z") => Some((|| {
                let left = Self::string_arg(heap, args, 0)?;
                let right = args
                    .get(1)
                    .and_then(|value| value.as_ref())
                    .and_then(|reference| heap.string(reference));
                Ok(Value::Int(i32::from(right == Some(left.as_str()))))
            })()),
            ("startsWith", "(Ljava/lang/String;)Z") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let prefix = Self::string_arg(heap, args, 1)?;
                Ok(Value::Int(i32::from(text.starts_with(&prefix))))
            })()),
            ("startsWith", "(Ljava/lang/String;I)Z") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let prefix = Self::string_arg(heap, args, 1)?;
                let offset = usize::try_from(Self::int_arg(args, 2)?)
                    .map_err(|_| "offset negatif".to_string())?;
                let rest = Self::utf16_slice(&text, offset, None);
                Ok(Value::Int(i32::from(rest.starts_with(&prefix))))
            })()),
            ("endsWith", "(Ljava/lang/String;)Z") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let suffix = Self::string_arg(heap, args, 1)?;
                Ok(Value::Int(i32::from(text.ends_with(&suffix))))
            })()),
            ("contains", "(Ljava/lang/CharSequence;)Z") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let needle = args
                    .get(1)
                    .copied()
                    .and_then(|value| Self::char_sequence_text(heap, value))
                    .ok_or("contains butuh CharSequence")?;
                Ok(Value::Int(i32::from(text.contains(&needle))))
            })()),
            // Java membuang karakter kontrol dan spasi, bukan semua whitespace Unicode.
            ("trim", "()Ljava/lang/String;") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let trimmed = text.trim_matches(|c: char| c <= ' ');
                Ok(heap.alloc(Object::String(trimmed.to_string())))
            })()),
            ("toLowerCase", "()Ljava/lang/String;")
            | ("toLowerCase", "(Ljava/util/Locale;)Ljava/lang/String;") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                Ok(heap.alloc(Object::String(text.to_lowercase())))
            })()),
            ("toUpperCase", "()Ljava/lang/String;")
            | ("toUpperCase", "(Ljava/util/Locale;)Ljava/lang/String;") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                Ok(heap.alloc(Object::String(text.to_uppercase())))
            })()),
            ("replace", "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Ljava/lang/String;") => {
                Some((|| {
                    let text = Self::string_arg(heap, args, 0)?;
                    let from = args
                        .get(1)
                        .copied()
                        .and_then(|value| Self::char_sequence_text(heap, value))
                        .ok_or("replace butuh CharSequence lama")?;
                    let to = args
                        .get(2)
                        .copied()
                        .and_then(|value| Self::char_sequence_text(heap, value))
                        .ok_or("replace butuh CharSequence baru")?;
                    Ok(heap.alloc(Object::String(text.replace(&from, &to))))
                })())
            }
            // Pola Java dan Rust sama untuk sebagian besar ekspresi. Konstruksi
            // yang tidak dikenal mesin Rust (lookaround, backreference) dilaporkan
            // sebagai error, bukan diganti diam-diam.
            ("replaceAll", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;")
            | ("replaceFirst", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;") => {
                Some((|| {
                    let text = Self::string_arg(heap, args, 0)?;
                    let pattern = Self::string_arg(heap, args, 1)?;
                    let replacement = Self::string_arg(heap, args, 2)?;
                    let compiled = regex::Regex::new(&pattern).map_err(|error| {
                        format!("pola regex belum didukung: {pattern}: {error}")
                    })?;
                    let result = if method == "replaceAll" {
                        compiled
                            .replace_all(&text, replacement.as_str())
                            .into_owned()
                    } else {
                        compiled.replace(&text, replacement.as_str()).into_owned()
                    };
                    Ok(heap.alloc(Object::String(result)))
                })())
            }
            // String.format bersifat statis: argumen pertama adalah pola.
            ("format", "(Ljava/lang/String;[Ljava/lang/Object;)Ljava/lang/String;")
            | (
                "format",
                "(Ljava/util/Locale;Ljava/lang/String;[Ljava/lang/Object;)Ljava/lang/String;",
            ) => Some((|| {
                let offset = if signature.starts_with("(Ljava/util/Locale;") {
                    1
                } else {
                    0
                };
                let pattern = Self::string_arg(heap, args, offset)?;
                let values = args
                    .get(offset + 1)
                    .and_then(|value| value.as_ref())
                    .and_then(|array| match heap.get(array) {
                        Some(Object::Array { elements, .. }) => Some(elements.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                Ok(heap.alloc(Object::String(Self::format_java(heap, &pattern, &values)?)))
            })()),
            ("compareTo", "(Ljava/lang/String;)I") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let other = Self::string_arg(heap, args, 1)?;
                let left: Vec<u16> = text.encode_utf16().collect();
                let right: Vec<u16> = other.encode_utf16().collect();
                for (a, b) in left.iter().zip(right.iter()) {
                    if a != b {
                        return Ok(Value::Int(i32::from(*a) - i32::from(*b)));
                    }
                }
                Ok(Value::Int(left.len() as i32 - right.len() as i32))
            })()),
            // Android memakai UTF-8 sebagai charset bawaan.
            ("getBytes", "()[B")
            | ("getBytes", "(Ljava/lang/String;)[B")
            | ("getBytes", "(Ljava/nio/charset/Charset;)[B") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let elements = text
                    .as_bytes()
                    .iter()
                    .map(|byte| Value::Int(i32::from(*byte)))
                    .collect();
                Ok(heap.alloc(Object::Array {
                    element_type: "B".to_string(),
                    elements,
                }))
            })()),
            ("contentEquals", "(Ljava/lang/CharSequence;)Z") => Some((|| {
                let left = args
                    .first()
                    .copied()
                    .and_then(|value| Self::char_sequence_text(heap, value))
                    .ok_or_else(|| {
                        let receiver = args.first().copied();
                        let object = receiver
                            .and_then(|value| value.as_ref())
                            .and_then(|reference| heap.get(reference));
                        format!(
                            "penerima contentEquals bukan CharSequence: {receiver:?} {object:?}"
                        )
                    })?;
                let right = args
                    .get(1)
                    .copied()
                    .and_then(|value| Self::char_sequence_text(heap, value));
                Ok(Value::Int(i32::from(
                    right.as_deref() == Some(left.as_str()),
                )))
            })()),
            ("concat", "(Ljava/lang/String;)Ljava/lang/String;") => Some((|| {
                let left = Self::string_arg(heap, args, 0)?;
                let right = Self::string_arg(heap, args, 1)?;
                Ok(heap.alloc(Object::String(format!("{left}{right}"))))
            })()),
            ("substring", "(I)Ljava/lang/String;") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let begin = Self::int_arg(args, 1)?;
                let begin = usize::try_from(begin).map_err(|_| "indeks negatif".to_string())?;
                Ok(heap.alloc(Object::String(Self::utf16_slice(&text, begin, None))))
            })()),
            ("indexOf", "(Ljava/lang/String;)I") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let needle = Self::string_arg(heap, args, 1)?;
                let result = match text.find(&needle) {
                    Some(byte_index) => text[..byte_index].encode_utf16().count() as i32,
                    None => -1,
                };
                Ok(Value::Int(result))
            })()),
            _ => None,
        }
    }

    fn builder_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                Self::set_builder_text(heap, reference, String::new())?;
                Ok(Value::Int(0))
            })()),
            ("<init>", "(Ljava/lang/String;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let text = Self::string_arg(heap, args, 1)?;
                Self::set_builder_text(heap, reference, text)?;
                Ok(Value::Int(0))
            })()),
            // Kapasitas awal tidak mengubah isi: penyimpanan memakai String.
            ("<init>", "(I)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                Self::set_builder_text(heap, reference, String::new())?;
                Ok(Value::Int(0))
            })()),
            ("<init>", "(Ljava/lang/CharSequence;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let text =
                    Self::char_sequence_text(heap, args.get(1).copied().unwrap_or(Value::Int(0)))
                        .unwrap_or_default();
                Self::set_builder_text(heap, reference, text)?;
                Ok(Value::Int(0))
            })()),
            ("append", "(Ljava/lang/String;)Ljava/lang/StringBuilder;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let mut text = Self::builder_text(heap, reference)
                    .ok_or_else(|| "StringBuilder belum diinisialisasi".to_string())?;
                text.push_str(&Self::string_arg(heap, args, 1)?);
                Self::set_builder_text(heap, reference, text)?;
                Ok(Value::Ref(reference))
            })()),
            ("append", "(I)Ljava/lang/StringBuilder;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let value = Self::int_arg(args, 1)?;
                let mut text = Self::builder_text(heap, reference)
                    .ok_or_else(|| "StringBuilder belum diinisialisasi".to_string())?;
                text.push_str(&value.to_string());
                Self::set_builder_text(heap, reference, text)?;
                Ok(Value::Ref(reference))
            })()),
            ("append", "(J)Ljava/lang/StringBuilder;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let value = args
                    .get(1)
                    .and_then(|value| value.as_long())
                    .ok_or("append butuh long")?;
                let mut text = Self::builder_text(heap, reference)
                    .ok_or_else(|| "StringBuilder belum diinisialisasi".to_string())?;
                text.push_str(&value.to_string());
                Self::set_builder_text(heap, reference, text)?;
                Ok(Value::Ref(reference))
            })()),
            ("append", "(C)Ljava/lang/StringBuilder;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let value = Self::int_arg(args, 1)?;
                let character = u32::try_from(value)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| format!("bukan karakter: {value}"))?;
                let mut text = Self::builder_text(heap, reference)
                    .ok_or_else(|| "StringBuilder belum diinisialisasi".to_string())?;
                text.push(character);
                Self::set_builder_text(heap, reference, text)?;
                Ok(Value::Ref(reference))
            })()),
            ("append", "(Z)Ljava/lang/StringBuilder;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let value = Self::int_arg(args, 1)?;
                let mut text = Self::builder_text(heap, reference)
                    .ok_or_else(|| "StringBuilder belum diinisialisasi".to_string())?;
                text.push_str(if value == 0 { "false" } else { "true" });
                Self::set_builder_text(heap, reference, text)?;
                Ok(Value::Ref(reference))
            })()),
            ("append", "(Ljava/lang/Object;)Ljava/lang/StringBuilder;")
            | ("append", "(Ljava/lang/CharSequence;)Ljava/lang/StringBuilder;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let mut text = Self::builder_text(heap, reference)
                    .ok_or_else(|| "StringBuilder belum diinisialisasi".to_string())?;
                let value = args.get(1).copied().unwrap_or(Value::Int(0));
                text.push_str(&Self::object_text(heap, value));
                Self::set_builder_text(heap, reference, text)?;
                Ok(Value::Ref(reference))
            })(
            )),
            ("append", "(F)Ljava/lang/StringBuilder;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let value = args
                    .get(1)
                    .and_then(|value| value.as_float())
                    .ok_or("append butuh float")?;
                let mut text = Self::builder_text(heap, reference)
                    .ok_or_else(|| "StringBuilder belum diinisialisasi".to_string())?;
                text.push_str(&Self::float_text_f32(value));
                Self::set_builder_text(heap, reference, text)?;
                Ok(Value::Ref(reference))
            })()),
            ("append", "(D)Ljava/lang/StringBuilder;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let value = args
                    .get(1)
                    .and_then(|value| value.as_double())
                    .ok_or("append butuh double")?;
                let mut text = Self::builder_text(heap, reference)
                    .ok_or_else(|| "StringBuilder belum diinisialisasi".to_string())?;
                text.push_str(&Self::float_text_f64(value));
                Self::set_builder_text(heap, reference, text)?;
                Ok(Value::Ref(reference))
            })()),
            ("toString", "()Ljava/lang/String;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let text = Self::builder_text(heap, reference)
                    .ok_or_else(|| "StringBuilder belum diinisialisasi".to_string())?;
                Ok(heap.alloc(Object::String(text)))
            })()),
            ("length", "()I") => Some((|| {
                let reference = Self::this_ref(args)?;
                let text = Self::builder_text(heap, reference)
                    .ok_or_else(|| "StringBuilder belum diinisialisasi".to_string())?;
                Ok(Value::Int(text.encode_utf16().count() as i32))
            })()),
            _ => None,
        }
    }

    /// `android.os.Looper` minimal: identitas main looper tanpa antrean nyata.
    ///
    /// Runtime ini berjalan pada satu thread, jadi main looper dan looper
    /// pemanggil diwakili objek yang sama bentuknya.
    fn looper_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        _args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getMainLooper", "()Landroid/os/Looper;") | ("myLooper", "()Landroid/os/Looper;") => {
                Some(Ok(Self::new_looper(heap)))
            }
            ("prepare", "()V") | ("prepareMainLooper", "()V") => Some(Ok(Value::Int(0))),
            // Runtime ini kooperatif dan satu thread. Memblokir mustahil, dan
            // kembali segera membuat aplikasi berputar pada
            // `while (true) { Looper.loop(); }`. Karena tidak ada tugas
            // tertunda, loop utama dilaporkan idle agar engine berhenti di
            // titik itu alih-alih membakar batas langkah.
            ("loop", "()V") => Some(Err(arunika_dex_engine::MAIN_LOOP_IDLE.to_string())),
            ("quit", "()V") | ("quitSafely", "()V") => Some(Ok(Value::Int(0))),
            ("isCurrentThread", "()Z") => Some(Ok(Value::Int(1))),
            _ => None,
        }
    }

    /// `android.os.Handler`: `post` mencatat Runnable untuk dijalankan engine
    /// sekali setelah lifecycle. Tidak ada penundaan waktu atau antrean pesan
    /// bertipe `Message`.
    fn handler_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "()V")
            | ("<init>", "(Landroid/os/Looper;)V")
            | ("<init>", "(Landroid/os/Looper;Landroid/os/Handler$Callback;)V")
            | ("<init>", "(Landroid/os/Handler$Callback;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                Self::reset_queue(heap, "Landroid/os/Handler;", reference)?;
                Ok(Value::Int(0))
            })()),
            ("post", "(Ljava/lang/Runnable;)Z")
            | ("postDelayed", "(Ljava/lang/Runnable;J)Z")
            | ("postAtFrontOfQueue", "(Ljava/lang/Runnable;)Z")
            | ("postAtTime", "(Ljava/lang/Runnable;J)Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                let runnable = args.get(1).copied().ok_or("post butuh Runnable")?;
                Self::push_to_queue(heap, "Landroid/os/Handler;", reference, runnable)?;
                Ok(Value::Int(1))
            })()),
            ("removeCallbacks", "(Ljava/lang/Runnable;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let runnable = args.get(1).copied().unwrap_or(Value::Int(0));
                Self::remove_from_queue(heap, "Landroid/os/Handler;", reference, runnable)?;
                Ok(Value::Int(0))
            })()),
            ("removeCallbacksAndMessages", "(Ljava/lang/Object;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                Self::clear_queue(heap, "Landroid/os/Handler;", reference)?;
                Ok(Value::Int(0))
            })()),
            ("getLooper", "()Landroid/os/Looper;") => Some(Ok(Self::new_looper(heap))),
            ("hasMessages", "(I)Z") | ("hasCallbacks", "(Ljava/lang/Runnable;)Z") => {
                Some(Ok(Value::Int(0)))
            }
            _ => None,
        }
    }

    fn new_looper(heap: &mut Heap) -> Value {
        heap.alloc(Object::Instance {
            type_name: "Landroid/os/Looper;".to_owned(),
            fields: std::collections::HashMap::new(),
        })
    }

    /// Siapkan antrean kosong pada field 1 milik instance `owner`.
    fn reset_queue(heap: &mut Heap, owner: &str, reference: u32) -> Result<(), String> {
        let list = heap.alloc(Object::List {
            type_name: "Ljava/util/ArrayList;".to_owned(),
            elements: Vec::new(),
        });
        match heap.get_mut(reference) {
            Some(Object::Instance { type_name, fields }) if type_name == owner => {
                fields.insert(0, Value::Int(0));
                fields.insert(1, list);
                Ok(())
            }
            _ => Err(format!("objek bukan {owner}")),
        }
    }

    fn push_to_queue(
        heap: &mut Heap,
        owner: &str,
        reference: u32,
        task: Value,
    ) -> Result<(), String> {
        let list = match heap.get(reference) {
            Some(Object::Instance { type_name, fields }) if type_name == owner => {
                fields.get(&1).and_then(|value| value.as_ref())
            }
            _ => return Err(format!("objek bukan {owner}")),
        };
        match list.and_then(|index| heap.get_mut(index)) {
            Some(Object::List { elements, .. }) => {
                elements.push(task);
                Ok(())
            }
            _ => Err("antrean tugas tidak tersedia".to_string()),
        }
    }

    fn remove_from_queue(
        heap: &mut Heap,
        owner: &str,
        reference: u32,
        task: Value,
    ) -> Result<(), String> {
        let list = match heap.get(reference) {
            Some(Object::Instance { type_name, fields }) if type_name == owner => {
                fields.get(&1).and_then(|value| value.as_ref())
            }
            _ => return Err(format!("objek bukan {owner}")),
        };
        match list.and_then(|index| heap.get_mut(index)) {
            Some(Object::List { elements, .. }) => {
                elements.retain(|candidate| *candidate != task);
                Ok(())
            }
            _ => Err("antrean tugas tidak tersedia".to_string()),
        }
    }

    fn clear_queue(heap: &mut Heap, owner: &str, reference: u32) -> Result<(), String> {
        let list = match heap.get(reference) {
            Some(Object::Instance { type_name, fields }) if type_name == owner => {
                fields.get(&1).and_then(|value| value.as_ref())
            }
            _ => return Err(format!("objek bukan {owner}")),
        };
        match list.and_then(|index| heap.get_mut(index)) {
            Some(Object::List { elements, .. }) => {
                elements.clear();
                Ok(())
            }
            _ => Err("antrean tugas tidak tersedia".to_string()),
        }
    }

    /// Executor kooperatif: tugas diantrekan dan dijalankan engine setelah
    /// lifecycle, bukan oleh thread pekerja.
    fn executors_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        _args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("newFixedThreadPool", "(I)Ljava/util/concurrent/ExecutorService;")
            | ("newSingleThreadExecutor", "()Ljava/util/concurrent/ExecutorService;")
            | ("newCachedThreadPool", "()Ljava/util/concurrent/ExecutorService;")
            | ("newWorkStealingPool", "()Ljava/util/concurrent/ExecutorService;")
            | ("newWorkStealingPool", "(I)Ljava/util/concurrent/ExecutorService;")
            | ("newScheduledThreadPool", "(I)Ljava/util/concurrent/ScheduledExecutorService;")
            | (
                "newSingleThreadScheduledExecutor",
                "()Ljava/util/concurrent/ScheduledExecutorService;",
            ) => Some(Ok(Self::new_executor(heap))),
            _ => None,
        }
    }

    /// `ExecutorService`: `execute`/`submit`/`schedule` mencatat tugas, lalu
    /// engine menjalankannya sekali. Argumen waktu tidak memengaruhi eksekusi.
    fn executor_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        const OWNER: &str = "Ljava/util/concurrent/ExecutorService;";
        match (method, signature) {
            ("execute", "(Ljava/lang/Runnable;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let task = args.get(1).copied().ok_or("execute butuh Runnable")?;
                Self::push_to_queue(heap, OWNER, reference, task)?;
                Ok(Value::Int(0))
            })()),
            ("submit", "(Ljava/lang/Runnable;)Ljava/util/concurrent/Future;")
            | ("submit", "(Ljava/lang/Runnable;Ljava/lang/Object;)Ljava/util/concurrent/Future;")
            | ("submit", "(Ljava/util/concurrent/Callable;)Ljava/util/concurrent/Future;") => {
                Some((|| {
                    let reference = Self::this_ref(args)?;
                    let task = args.get(1).copied().ok_or("submit butuh tugas")?;
                    Self::push_to_queue(heap, OWNER, reference, task)?;
                    Ok(Self::new_future(heap))
                })())
            }
            (
                "schedule",
                "(Ljava/lang/Runnable;JLjava/util/concurrent/TimeUnit;)Ljava/util/concurrent/ScheduledFuture;",
            )
            | (
                "schedule",
                "(Ljava/util/concurrent/Callable;JLjava/util/concurrent/TimeUnit;)Ljava/util/concurrent/ScheduledFuture;",
            )
            | (
                "scheduleAtFixedRate",
                "(Ljava/lang/Runnable;JJLjava/util/concurrent/TimeUnit;)Ljava/util/concurrent/ScheduledFuture;",
            )
            | (
                "scheduleWithFixedDelay",
                "(Ljava/lang/Runnable;JJLjava/util/concurrent/TimeUnit;)Ljava/util/concurrent/ScheduledFuture;",
            ) => Some((|| {
                let reference = Self::this_ref(args)?;
                let task = args.get(1).copied().ok_or("schedule butuh tugas")?;
                Self::push_to_queue(heap, OWNER, reference, task)?;
                Ok(Self::new_future(heap))
            })()),
            ("shutdown", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                Self::mark_shutdown(heap, OWNER, reference)?;
                Ok(Value::Int(0))
            })()),
            ("shutdownNow", "()Ljava/util/List;") => Some((|| {
                let reference = Self::this_ref(args)?;
                Self::mark_shutdown(heap, OWNER, reference)?;
                Self::clear_queue(heap, OWNER, reference)?;
                Ok(heap.alloc(Object::List {
                    type_name: "Ljava/util/ArrayList;".to_owned(),
                    elements: Vec::new(),
                }))
            })()),
            ("isShutdown", "()Z") | ("isTerminated", "()Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                let flag = match heap.get(reference) {
                    Some(Object::Instance { fields, .. }) => {
                        fields.get(&0).and_then(|value| value.as_int()).unwrap_or(0)
                    }
                    _ => 0,
                };
                Ok(Value::Int(i32::from(flag != 0)))
            })()),
            ("awaitTermination", "(JLjava/util/concurrent/TimeUnit;)Z") => Some(Ok(Value::Int(1))),
            _ => None,
        }
    }

    /// `Future` minimal: tugas kooperatif sudah selesai saat ditanyakan.
    fn future_method(
        _heap: &mut Heap,
        method: &str,
        signature: &str,
        _args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("get", "()Ljava/lang/Object;")
            | ("get", "(JLjava/util/concurrent/TimeUnit;)Ljava/lang/Object;") => {
                Some(Ok(Value::Int(0)))
            }
            ("isDone", "()Z") | ("isCancelled", "()Z") => Some(Ok(Value::Int(1))),
            ("cancel", "(Z)Z") => Some(Ok(Value::Int(1))),
            _ => None,
        }
    }

    fn new_executor(heap: &mut Heap) -> Value {
        let reference = heap.alloc(Object::Instance {
            type_name: "Ljava/util/concurrent/ExecutorService;".to_owned(),
            fields: std::collections::HashMap::new(),
        });
        if let Some(index) = reference.as_ref() {
            let _ = Self::reset_queue(heap, "Ljava/util/concurrent/ExecutorService;", index);
        }
        reference
    }

    fn new_future(heap: &mut Heap) -> Value {
        heap.alloc(Object::Instance {
            type_name: "Ljava/util/concurrent/Future;".to_owned(),
            fields: std::collections::HashMap::new(),
        })
    }

    fn mark_shutdown(heap: &mut Heap, owner: &str, reference: u32) -> Result<(), String> {
        match heap.get_mut(reference) {
            Some(Object::Instance { type_name, fields }) if type_name == owner => {
                fields.insert(0, Value::Int(1));
                Ok(())
            }
            _ => Err(format!("objek bukan {owner}")),
        }
    }

    /// Timer kooperatif.
    ///
    /// Tugas hanya dicatat di sini; engine menjalankannya sekali lewat
    /// `run_due_timers` setelah lifecycle selesai. Tidak ada jam dinding,
    /// thread penjadwal, atau pengulangan periodik, jadi argumen delay dan
    /// period tidak memengaruhi waktu eksekusi.
    fn timer_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "()V")
            | ("<init>", "(Z)V")
            | ("<init>", "(Ljava/lang/String;)V")
            | ("<init>", "(Ljava/lang/String;Z)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                Self::reset_queue(heap, "Ljava/util/Timer;", reference)?;
                Ok(Value::Int(0))
            })()),
            ("schedule", "(Ljava/util/TimerTask;J)V")
            | ("schedule", "(Ljava/util/TimerTask;JJ)V")
            | ("scheduleAtFixedRate", "(Ljava/util/TimerTask;J)V")
            | ("scheduleAtFixedRate", "(Ljava/util/TimerTask;JJ)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let task = args.get(1).copied().ok_or("schedule butuh TimerTask")?;
                Self::push_to_queue(heap, "Ljava/util/Timer;", reference, task)?;
                Ok(Value::Int(0))
            })()),
            ("cancel", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get_mut(reference) {
                    Some(Object::Instance { type_name, fields })
                        if type_name == "Ljava/util/Timer;" =>
                    {
                        fields.insert(0, Value::Int(1));
                        fields.insert(1, Value::Int(0));
                        Ok(Value::Int(0))
                    }
                    _ => Err("objek bukan Timer".to_string()),
                }
            })()),
            ("purge", "()I") => Some(Ok(Value::Int(0))),
            _ => None,
        }
    }

    /// Anggota `TimerTask` yang tidak membutuhkan penjadwal.
    fn timer_task_method(
        _heap: &mut Heap,
        method: &str,
        signature: &str,
        _args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "()V") => Some(Ok(Value::Int(0))),
            ("cancel", "()V") => Some(Ok(Value::Int(0))),
            ("scheduledExecutionTime", "()J") => Some(Ok(Value::from_long(0))),
            _ => None,
        }
    }

    fn boxed_value(heap: &Heap, args: &[Value]) -> Result<Value, String> {
        let reference = Self::this_ref(args)?;
        match heap.get(reference) {
            Some(Object::Boxed { value, .. }) => Ok(*value),
            Some(_) => Err("objek bukan nilai boxed".to_string()),
            None => Err("objek boxed tidak ada di heap".to_string()),
        }
    }

    /// Koleksi `java.util` yang disediakan runtime.
    ///
    /// `new-instance` sudah membuat objek instance kosong; `<init>` mengganti
    /// objek itu menjadi `Object::List` sehingga operasi koleksi punya tempat
    /// penyimpanan elemen yang jelas.
    fn list_method(
        heap: &mut Heap,
        class_descriptor: &str,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get_mut(reference) {
                    Some(slot) => {
                        *slot = Object::List {
                            type_name: class_descriptor.to_owned(),
                            elements: Vec::new(),
                        };
                        Ok(Value::Int(0))
                    }
                    None => Err("objek koleksi tidak ada di heap".to_string()),
                }
            })()),
            ("add", "(Ljava/lang/Object;)Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                let value = args.get(1).copied().ok_or("add butuh satu argumen")?;
                match heap.get_mut(reference) {
                    Some(Object::List { elements, .. }) => {
                        elements.push(value);
                        Ok(Value::Int(1))
                    }
                    Some(_) => Err("objek bukan koleksi".to_string()),
                    None => Err("objek koleksi tidak ada di heap".to_string()),
                }
            })()),
            ("get", "(I)Ljava/lang/Object;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let index = Self::int_arg(args, 1)?;
                match heap.get(reference) {
                    Some(Object::List { elements, .. }) => {
                        let slot = usize::try_from(index)
                            .ok()
                            .and_then(|slot| elements.get(slot))
                            .copied()
                            .ok_or_else(|| {
                                format!("indeks {index} di luar panjang {}", elements.len())
                            })?;
                        Ok(slot)
                    }
                    Some(_) => Err("objek bukan koleksi".to_string()),
                    None => Err("objek koleksi tidak ada di heap".to_string()),
                }
            })()),
            ("set", "(ILjava/lang/Object;)Ljava/lang/Object;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let index = Self::int_arg(args, 1)?;
                let value = args.get(2).copied().ok_or("set butuh dua argumen")?;
                match heap.get_mut(reference) {
                    Some(Object::List { elements, .. }) => {
                        let length = elements.len();
                        let slot = usize::try_from(index)
                            .ok()
                            .and_then(|slot| elements.get_mut(slot))
                            .ok_or_else(|| format!("indeks {index} di luar panjang {length}"))?;
                        Ok(std::mem::replace(slot, value))
                    }
                    Some(_) => Err("objek bukan koleksi".to_string()),
                    None => Err("objek koleksi tidak ada di heap".to_string()),
                }
            })()),
            ("remove", "(I)Ljava/lang/Object;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let index = Self::int_arg(args, 1)?;
                match heap.get_mut(reference) {
                    Some(Object::List { elements, .. }) => {
                        let length = elements.len();
                        let slot = usize::try_from(index)
                            .ok()
                            .filter(|slot| *slot < length)
                            .ok_or_else(|| format!("indeks {index} di luar panjang {length}"))?;
                        Ok(elements.remove(slot))
                    }
                    Some(_) => Err("objek bukan koleksi".to_string()),
                    None => Err("objek koleksi tidak ada di heap".to_string()),
                }
            })()),
            ("size", "()I") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get(reference) {
                    Some(Object::List { elements, .. }) => Ok(Value::Int(elements.len() as i32)),
                    Some(_) => Err("objek bukan koleksi".to_string()),
                    None => Err("objek koleksi tidak ada di heap".to_string()),
                }
            })()),
            ("isEmpty", "()Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get(reference) {
                    Some(Object::List { elements, .. }) => {
                        Ok(Value::Int(i32::from(elements.is_empty())))
                    }
                    Some(_) => Err("objek bukan koleksi".to_string()),
                    None => Err("objek koleksi tidak ada di heap".to_string()),
                }
            })()),
            ("clear", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get_mut(reference) {
                    Some(Object::List { elements, .. }) => {
                        elements.clear();
                        Ok(Value::Int(0))
                    }
                    Some(_) => Err("objek bukan koleksi".to_string()),
                    None => Err("objek koleksi tidak ada di heap".to_string()),
                }
            })()),
            _ => None,
        }
    }

    /// Perbandingan kunci map: String/boxed dibandingkan isinya, objek lain
    /// dibandingkan identitasnya seperti `equals` bawaan.
    fn values_equal(heap: &Heap, left: Value, right: Value) -> bool {
        if left == right {
            return true;
        }
        match (left, right) {
            (Value::Ref(a), Value::Ref(b)) => match (heap.get(a), heap.get(b)) {
                (Some(Object::String(x)), Some(Object::String(y))) => x == y,
                (Some(Object::Boxed { value: x, .. }), Some(Object::Boxed { value: y, .. })) => {
                    x == y
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn map_method(
        heap: &mut Heap,
        class_descriptor: &str,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get_mut(reference) {
                    Some(slot) => {
                        *slot = Object::Map {
                            type_name: class_descriptor.to_owned(),
                            entries: Vec::new(),
                        };
                        Ok(Value::Int(0))
                    }
                    None => Err("objek map tidak ada di heap".to_string()),
                }
            })()),
            ("put", "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let key = args.get(1).copied().ok_or("put butuh kunci")?;
                let value = args.get(2).copied().ok_or("put butuh nilai")?;
                let existing = match heap.get(reference) {
                    Some(Object::Map { entries, .. }) => entries
                        .iter()
                        .position(|(candidate, _)| Self::values_equal(heap, *candidate, key)),
                    Some(_) => return Err("objek bukan map".to_string()),
                    None => return Err("objek map tidak ada di heap".to_string()),
                };
                match heap.get_mut(reference) {
                    Some(Object::Map { entries, .. }) => match existing {
                        Some(slot) => Ok(std::mem::replace(&mut entries[slot].1, value)),
                        None => {
                            entries.push((key, value));
                            Ok(Value::Int(0))
                        }
                    },
                    _ => Err("objek bukan map".to_string()),
                }
            })(
            )),
            ("get", "(Ljava/lang/Object;)Ljava/lang/Object;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let key = args.get(1).copied().ok_or("get butuh kunci")?;
                match heap.get(reference) {
                    Some(Object::Map { entries, .. }) => Ok(entries
                        .iter()
                        .find(|(candidate, _)| Self::values_equal(heap, *candidate, key))
                        .map(|(_, value)| *value)
                        .unwrap_or(Value::Int(0))),
                    Some(_) => Err("objek bukan map".to_string()),
                    None => Err("objek map tidak ada di heap".to_string()),
                }
            })()),
            ("containsKey", "(Ljava/lang/Object;)Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                let key = args.get(1).copied().ok_or("containsKey butuh kunci")?;
                match heap.get(reference) {
                    Some(Object::Map { entries, .. }) => {
                        Ok(Value::Int(i32::from(entries.iter().any(
                            |(candidate, _)| Self::values_equal(heap, *candidate, key),
                        ))))
                    }
                    Some(_) => Err("objek bukan map".to_string()),
                    None => Err("objek map tidak ada di heap".to_string()),
                }
            })()),
            ("remove", "(Ljava/lang/Object;)Ljava/lang/Object;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let key = args.get(1).copied().ok_or("remove butuh kunci")?;
                let existing = match heap.get(reference) {
                    Some(Object::Map { entries, .. }) => entries
                        .iter()
                        .position(|(candidate, _)| Self::values_equal(heap, *candidate, key)),
                    Some(_) => return Err("objek bukan map".to_string()),
                    None => return Err("objek map tidak ada di heap".to_string()),
                };
                match heap.get_mut(reference) {
                    Some(Object::Map { entries, .. }) => match existing {
                        Some(slot) => Ok(entries.remove(slot).1),
                        None => Ok(Value::Int(0)),
                    },
                    _ => Err("objek bukan map".to_string()),
                }
            })()),
            ("size", "()I") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get(reference) {
                    Some(Object::Map { entries, .. }) => Ok(Value::Int(entries.len() as i32)),
                    Some(_) => Err("objek bukan map".to_string()),
                    None => Err("objek map tidak ada di heap".to_string()),
                }
            })()),
            ("isEmpty", "()Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get(reference) {
                    Some(Object::Map { entries, .. }) => {
                        Ok(Value::Int(i32::from(entries.is_empty())))
                    }
                    Some(_) => Err("objek bukan map".to_string()),
                    None => Err("objek map tidak ada di heap".to_string()),
                }
            })()),
            ("clear", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get_mut(reference) {
                    Some(Object::Map { entries, .. }) => {
                        entries.clear();
                        Ok(Value::Int(0))
                    }
                    Some(_) => Err("objek bukan map".to_string()),
                    None => Err("objek map tidak ada di heap".to_string()),
                }
            })()),
            _ => None,
        }
    }

    fn class_descriptor(heap: &Heap, args: &[Value]) -> Result<String, String> {
        let reference = Self::this_ref(args)?;
        match heap.get(reference) {
            Some(Object::Class { descriptor }) => Ok(descriptor.clone()),
            _ => Err("objek bukan java.lang.Class".to_string()),
        }
    }

    /// Nama Java dari descriptor; array dan primitif dikembalikan apa adanya.
    fn java_name(descriptor: &str) -> String {
        match descriptor
            .strip_prefix('L')
            .and_then(|rest| rest.strip_suffix(';'))
        {
            Some(class) => class.replace('/', "."),
            None => descriptor.to_owned(),
        }
    }

    fn class_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getName", "()Ljava/lang/String;") => Some((|| {
                let name = Self::java_name(&Self::class_descriptor(heap, args)?);
                Ok(heap.alloc(Object::String(name)))
            })()),
            ("forName", "(Ljava/lang/String;)Ljava/lang/Class;")
            | ("forName", "(Ljava/lang/String;ZLjava/lang/ClassLoader;)Ljava/lang/Class;") => {
                Some((|| {
                    let name = Self::string_arg(heap, args, 0)?;
                    Ok(heap.alloc(Object::Class {
                        descriptor: Self::descriptor_from_java_name(&name),
                    }))
                })())
            }
            // Nama kanonik sama dengan nama Java untuk kelas non-array; bentuk
            // array seperti "java.lang.String[]" belum dibedakan.
            ("getCanonicalName", "()Ljava/lang/String;") => Some((|| {
                let name = Self::java_name(&Self::class_descriptor(heap, args)?);
                Ok(heap.alloc(Object::String(name)))
            })()),
            ("getSimpleName", "()Ljava/lang/String;") => Some((|| {
                let name = Self::java_name(&Self::class_descriptor(heap, args)?);
                let simple = name.rsplit('.').next().unwrap_or(&name).to_owned();
                Ok(heap.alloc(Object::String(simple)))
            })()),
            _ => None,
        }
    }

    /// `SharedPreferences` disimpan di memori selama proses berjalan.
    /// `edit()` mengembalikan objek yang sama sehingga `apply`/`commit` cukup
    /// melaporkan sukses tanpa menulis disk.
    fn prefs_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getString", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;") => {
                Some(Self::prefs_get(heap, args, 1, 2))
            }
            ("getInt", "(Ljava/lang/String;I)I") => Some(Self::prefs_get(heap, args, 1, 2)),
            ("getBoolean", "(Ljava/lang/String;Z)Z") => Some(Self::prefs_get(heap, args, 1, 2)),
            ("getLong", "(Ljava/lang/String;J)J") => Some(Self::prefs_get(heap, args, 1, 2)),
            ("getFloat", "(Ljava/lang/String;F)F") => Some(Self::prefs_get(heap, args, 1, 2)),
            ("contains", "(Ljava/lang/String;)Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                let key = args.get(1).copied().ok_or("contains butuh kunci")?;
                match heap.get(reference) {
                    Some(Object::Map { entries, .. }) => {
                        Ok(Value::Int(i32::from(entries.iter().any(
                            |(candidate, _)| Self::values_equal(heap, *candidate, key),
                        ))))
                    }
                    Some(_) => Err("objek bukan SharedPreferences".to_string()),
                    None => Err("SharedPreferences tidak ada di heap".to_string()),
                }
            })()),
            ("edit", "()Landroid/content/SharedPreferences$Editor;") => {
                Some(Self::this_ref(args).map(Value::Ref))
            }
            ("getAll", "()Ljava/util/Map;") => Some(Self::this_ref(args).map(Value::Ref)),
            (
                "registerOnSharedPreferenceChangeListener",
                "(Landroid/content/SharedPreferences$OnSharedPreferenceChangeListener;)V",
            )
            | (
                "unregisterOnSharedPreferenceChangeListener",
                "(Landroid/content/SharedPreferences$OnSharedPreferenceChangeListener;)V",
            ) => Some(Ok(Value::Int(0))),
            _ => None,
        }
    }

    fn prefs_get(
        heap: &mut Heap,
        args: &[Value],
        key_index: usize,
        default_index: usize,
    ) -> Result<Value, String> {
        let reference = Self::this_ref(args)?;
        let key = args.get(key_index).copied().ok_or("prefs butuh kunci")?;
        let default = args
            .get(default_index)
            .copied()
            .ok_or("prefs butuh nilai default")?;
        match heap.get(reference) {
            Some(Object::Map { entries, .. }) => Ok(entries
                .iter()
                .find(|(candidate, _)| Self::values_equal(heap, *candidate, key))
                .map(|(_, value)| *value)
                .unwrap_or(default)),
            Some(_) => Err("objek bukan SharedPreferences".to_string()),
            None => Err("SharedPreferences tidak ada di heap".to_string()),
        }
    }

    /// Editor memakai map yang sama dengan preferensinya.
    fn editor_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            (
                "putString",
                "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;",
            )
            | ("putInt", "(Ljava/lang/String;I)Landroid/content/SharedPreferences$Editor;")
            | ("putBoolean", "(Ljava/lang/String;Z)Landroid/content/SharedPreferences$Editor;")
            | ("putLong", "(Ljava/lang/String;J)Landroid/content/SharedPreferences$Editor;")
            | ("putFloat", "(Ljava/lang/String;F)Landroid/content/SharedPreferences$Editor;") => {
                Some((|| {
                    let reference = Self::this_ref(args)?;
                    let key = args.get(1).copied().ok_or("put butuh kunci")?;
                    let value = args.get(2).copied().ok_or("put butuh nilai")?;
                    let existing = match heap.get(reference) {
                        Some(Object::Map { entries, .. }) => entries
                            .iter()
                            .position(|(candidate, _)| Self::values_equal(heap, *candidate, key)),
                        Some(_) => return Err("objek bukan editor".to_string()),
                        None => return Err("editor tidak ada di heap".to_string()),
                    };
                    match heap.get_mut(reference) {
                        Some(Object::Map { entries, .. }) => {
                            match existing {
                                Some(slot) => entries[slot].1 = value,
                                None => entries.push((key, value)),
                            }
                            Ok(Value::Ref(reference))
                        }
                        _ => Err("objek bukan editor".to_string()),
                    }
                })())
            }
            ("remove", "(Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;") => {
                Some((|| {
                    let reference = Self::this_ref(args)?;
                    let key = args.get(1).copied().ok_or("remove butuh kunci")?;
                    let existing = match heap.get(reference) {
                        Some(Object::Map { entries, .. }) => entries
                            .iter()
                            .position(|(candidate, _)| Self::values_equal(heap, *candidate, key)),
                        Some(_) => return Err("objek bukan editor".to_string()),
                        None => return Err("editor tidak ada di heap".to_string()),
                    };
                    match heap.get_mut(reference) {
                        Some(Object::Map { entries, .. }) => {
                            if let Some(slot) = existing {
                                entries.remove(slot);
                            }
                            Ok(Value::Ref(reference))
                        }
                        _ => Err("objek bukan editor".to_string()),
                    }
                })())
            }
            ("clear", "()Landroid/content/SharedPreferences$Editor;") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get_mut(reference) {
                    Some(Object::Map { entries, .. }) => {
                        entries.clear();
                        Ok(Value::Ref(reference))
                    }
                    Some(_) => Err("objek bukan editor".to_string()),
                    None => Err("editor tidak ada di heap".to_string()),
                }
            })()),
            ("commit", "()Z") => Some(Ok(Value::Int(1))),
            ("apply", "()V") => Some(Ok(Value::Int(0))),
            _ => None,
        }
    }

    fn object_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "()V") => Some(Ok(Value::Int(0))),
            ("getClass", "()Ljava/lang/Class;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let descriptor = match heap.get(reference) {
                    Some(Object::Instance { type_name, .. })
                    | Some(Object::List { type_name, .. })
                    | Some(Object::Map { type_name, .. })
                    | Some(Object::Stream { type_name, .. })
                    | Some(Object::Boxed { type_name, .. }) => type_name.clone(),
                    Some(Object::Method { .. }) => "Ljava/lang/reflect/Method;".to_string(),
                    Some(Object::String(_)) => "Ljava/lang/String;".to_string(),
                    Some(Object::Class { .. }) => "Ljava/lang/Class;".to_string(),
                    Some(Object::Array { element_type, .. }) => format!("[{element_type}"),
                    Some(Object::Bitmap { .. }) => "Landroid/graphics/Bitmap;".to_string(),
                    None => return Err("objek tidak ada di heap".to_string()),
                };
                Ok(heap.alloc(Object::Class { descriptor }))
            })()),
            // Identitas objek dipetakan ke indeks heap: stabil selama proses,
            // sama seperti hashCode bawaan Java yang berbasis identitas.
            ("hashCode", "()I") => Some((|| {
                let reference = Self::this_ref(args)?;
                Ok(Value::Int(reference as i32))
            })()),
            ("equals", "(Ljava/lang/Object;)Z") => Some((|| {
                let left = Self::this_ref(args)?;
                let right = args.get(1).and_then(|value| value.as_ref());
                Ok(Value::Int(i32::from(right == Some(left))))
            })()),
            ("toString", "()Ljava/lang/String;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let descriptor = match heap.get(reference) {
                    Some(Object::Instance { type_name, .. })
                    | Some(Object::List { type_name, .. })
                    | Some(Object::Map { type_name, .. })
                    | Some(Object::Stream { type_name, .. })
                    | Some(Object::Boxed { type_name, .. }) => type_name.clone(),
                    Some(Object::Method { .. }) => "Ljava/lang/reflect/Method;".to_string(),
                    Some(Object::String(_)) => "Ljava/lang/String;".to_string(),
                    Some(Object::Class { .. }) => "Ljava/lang/Class;".to_string(),
                    Some(Object::Array { element_type, .. }) => format!("[{element_type}"),
                    Some(Object::Bitmap { .. }) => "Landroid/graphics/Bitmap;".to_string(),
                    None => return Err("objek tidak ada di heap".to_string()),
                };
                let text = format!("{}@{:x}", Self::java_name(&descriptor), reference);
                Ok(heap.alloc(Object::String(text)))
            })()),
            _ => None,
        }
    }

    fn app_file(
        heap: &mut Heap,
        app_dir: Option<&std::path::Path>,
        sub: &str,
    ) -> Result<Value, String> {
        match app_dir {
            Some(root) => Ok(Self::make_file(
                heap,
                root.join(sub).to_string_lossy().into_owned(),
            )),
            None => Ok(Value::Int(0)),
        }
    }

    fn file_path(heap: &Heap, args: &[Value]) -> Result<String, String> {
        let reference = Self::this_ref(args)?;
        match heap.get(reference) {
            Some(Object::Instance { fields, .. }) => fields
                .get(&0)
                .and_then(|value| value.as_ref())
                .and_then(|text| heap.string(text))
                .map(|text| text.to_owned())
                .ok_or_else(|| "File tanpa path".to_string()),
            Some(_) => Err("objek bukan File".to_string()),
            None => Err("objek File tidak ada di heap".to_string()),
        }
    }

    fn make_map(heap: &mut Heap, type_name: &str) -> Value {
        heap.alloc(Object::Map {
            type_name: type_name.to_string(),
            entries: Vec::new(),
        })
    }

    fn make_file(heap: &mut Heap, path: String) -> Value {
        let text = heap.alloc(Object::String(path));
        let mut fields = std::collections::HashMap::new();
        fields.insert(0u32, text);
        heap.alloc(Object::Instance {
            type_name: "Ljava/io/File;".to_string(),
            fields,
        })
    }

    /// `java.io.File`: hanya operasi path. Runtime belum membuka akses disk,
    /// sehingga `exists`/`mkdirs`/`length` melaporkan tidak ada berkas.
    fn file_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "(Ljava/lang/String;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let path = Self::string_arg(heap, args, 1)?;
                Self::set_file_path(heap, reference, path)?;
                Ok(Value::Int(0))
            })()),
            ("<init>", "(Ljava/io/File;Ljava/lang/String;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let parent = Self::string_arg(heap, args, 1)?;
                let child = Self::string_arg(heap, args, 2)?;
                Self::set_file_path(heap, reference, format!("{parent}/{child}"))?;
                Ok(Value::Int(0))
            })()),
            ("getPath", "()Ljava/lang/String;") | ("toString", "()Ljava/lang/String;") => {
                Some((|| {
                    let path = Self::file_path(heap, args)?;
                    Ok(heap.alloc(Object::String(path)))
                })())
            }
            ("getAbsolutePath", "()Ljava/lang/String;") => Some((|| {
                let path = Self::file_path(heap, args)?;
                let absolute = if path.starts_with('/') {
                    path
                } else {
                    format!("/{path}")
                };
                Ok(heap.alloc(Object::String(absolute)))
            })()),
            ("getAbsoluteFile", "()Ljava/io/File;") => Some((|| {
                let path = Self::file_path(heap, args)?;
                let absolute = if path.starts_with('/') {
                    path
                } else {
                    format!("/{path}")
                };
                Ok(Self::make_file(heap, absolute))
            })()),
            ("getName", "()Ljava/lang/String;") => Some((|| {
                let path = Self::file_path(heap, args)?;
                let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
                Ok(heap.alloc(Object::String(name)))
            })()),
            ("getParent", "()Ljava/lang/String;") => Some((|| {
                let path = Self::file_path(heap, args)?;
                match path.rfind('/') {
                    Some(0) => Ok(heap.alloc(Object::String("/".to_string()))),
                    Some(index) => Ok(heap.alloc(Object::String(path[..index].to_string()))),
                    None => Ok(Value::Int(0)),
                }
            })()),
            ("exists", "()Z")
            | ("isDirectory", "()Z")
            | ("isFile", "()Z")
            | ("mkdir", "()Z")
            | ("mkdirs", "()Z")
            | ("delete", "()Z") => Some(Ok(Value::Int(0))),
            ("length", "()J") => Some(Ok(Value::from_long(0))),
            _ => None,
        }
    }

    fn set_file_path(heap: &mut Heap, reference: u32, path: String) -> Result<(), String> {
        let text = heap.alloc(Object::String(path));
        match heap.get_mut(reference) {
            Some(Object::Instance { fields, .. }) => {
                fields.insert(0, text);
                Ok(())
            }
            _ => Err("objek bukan File".to_string()),
        }
    }

    /// `android.util.Log`: belum menulis ke mana pun, tetapi mengembalikan
    /// panjang pesan seperti kontrak Android.
    fn log_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("v" | "d" | "i" | "w" | "e" | "wtf", "(Ljava/lang/String;Ljava/lang/String;)I")
            | (
                "v" | "d" | "i" | "w" | "e" | "wtf",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/Throwable;)I",
            ) => Some((|| {
                let message = Self::string_arg(heap, args, 1)?;
                Ok(Value::Int(message.encode_utf16().count() as i32))
            })()),
            ("isLoggable", "(Ljava/lang/String;I)Z") => Some(Ok(Value::Int(1))),
            // Tanpa frame stack, yang bisa dilaporkan hanya kelas dan pesan.
            ("getStackTraceString", "(Ljava/lang/Throwable;)Ljava/lang/String;") => {
                let text = args
                    .first()
                    .and_then(|value| value.as_ref())
                    .and_then(|reference| heap.get(reference))
                    .map(|object| Self::describe_throwable(heap, object))
                    .unwrap_or_default();
                Some(Ok(heap.alloc(Object::String(text))))
            }
            _ => None,
        }
    }

    /// Method `android.app.Activity` yang belum punya efek samping di runtime.
    ///
    /// Ini bukan klaim dukungan Activity: tidak ada jendela, render, atau
    /// penyimpanan state. Method hanya diterima agar kode aplikasi yang
    /// memanggil lifecycle induk tidak gagal sebelum logika miliknya berjalan.
    fn activity_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        let app_dir = self.app_dir.as_deref();
        let package = self.manifest.package.as_deref();
        match (method, signature) {
            // Izin yang tidak dideklarasikan manifest dilaporkan ditolak, bukan
            // diberikan diam-diam: runtime belum punya dialog persetujuan.
            ("checkCallingOrSelfPermission", "(Ljava/lang/String;)I")
            | ("checkCallingPermission", "(Ljava/lang/String;)I")
            | ("checkSelfPermission", "(Ljava/lang/String;)I") => Some((|| {
                let name = Self::string_arg(heap, args, 1)?;
                let granted = self.permission_granted(&name);
                Ok(Value::Int(if granted { 0 } else { -1 }))
            })()),
            ("checkPermission", "(Ljava/lang/String;II)I") => Some((|| {
                let name = Self::string_arg(heap, args, 1)?;
                let granted = self.permission_granted(&name);
                Ok(Value::Int(if granted { 0 } else { -1 }))
            })()),
            (
                "getSharedPreferences",
                "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
            ) => Some((|| {
                let name = Self::string_arg(heap, args, 1)?;
                let mut cache = self
                    .prefs
                    .lock()
                    .map_err(|_| "cache preferensi rusak".to_string())?;
                if let Some(value) = cache.get(&name) {
                    return Ok(*value);
                }
                let map = Self::make_map(heap, "Landroid/content/SharedPreferences;");
                cache.insert(name, map);
                Ok(map)
            })()),
            // Path bersifat virtual: runtime belum melakukan I/O disk.
            // Activity sendiri berperan sebagai Context pada runtime ini.
            ("getApplicationContext", "()Landroid/content/Context;")
            | ("getBaseContext", "()Landroid/content/Context;") => {
                Some(Self::this_ref(args).map(Value::Ref))
            }
            ("getPackageName", "()Ljava/lang/String;") => Some(match package {
                Some(package) => Ok(heap.alloc(Object::String(package.to_string()))),
                None => Ok(Value::Int(0)),
            }),
            // Objek ini hanya membawa identitas; nilai field-nya dipasok
            // `field_default` saat kode aplikasi membacanya.
            ("getPackageManager", "()Landroid/content/pm/PackageManager;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/content/pm/PackageManager;"),
            )),
            ("getApplication", "()Landroid/app/Application;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/app/Application;"),
            )),
            ("getResources", "()Landroid/content/res/Resources;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/content/res/Resources;"),
            )),
            ("getAssets", "()Landroid/content/res/AssetManager;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/content/res/AssetManager;"),
            )),
            ("getContentResolver", "()Landroid/content/ContentResolver;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/content/ContentResolver;"),
            )),
            ("getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;") => Some((|| {
                let name = Self::string_arg(heap, args, 1)?;
                // Layanan yang tidak dikenal dilaporkan null, sama seperti
                // Context Android untuk nama layanan yang tidak ada.
                let descriptor = match name.as_str() {
                    "window" => "Landroid/view/WindowManager;",
                    "connectivity" => "Landroid/net/ConnectivityManager;",
                    "telephony" => "Landroid/telephony/TelephonyManager;",
                    "wifi" => "Landroid/net/wifi/WifiManager;",
                    "audio" => "Landroid/media/AudioManager;",
                    "activity" => "Landroid/app/ActivityManager;",
                    "power" => "Landroid/os/PowerManager;",
                    "vibrator" => "Landroid/os/Vibrator;",
                    "notification" => "Landroid/app/NotificationManager;",
                    "clipboard" => "Landroid/content/ClipboardManager;",
                    "layout_inflater" => "Landroid/view/LayoutInflater;",
                    _ => return Ok(Value::Int(0)),
                };
                Ok(Self::framework_instance(heap, descriptor))
            })(
            )),
            ("getWindowManager", "()Landroid/view/WindowManager;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/view/WindowManager;"),
            )),
            ("getFilesDir", "()Ljava/io/File;") => Some(Self::app_file(heap, app_dir, "files")),
            ("getCacheDir", "()Ljava/io/File;") | ("getExternalCacheDir", "()Ljava/io/File;") => {
                Some(Self::app_file(heap, app_dir, "cache"))
            }
            ("getExternalFilesDir", "(Ljava/lang/String;)Ljava/io/File;") => Some((|| {
                let Some(root) = app_dir else {
                    return Ok(Value::Int(0));
                };
                let sub = args
                    .get(1)
                    .and_then(|value| value.as_ref())
                    .and_then(|text| heap.string(text))
                    .map(|text| text.to_owned());
                let mut path = root.join("files");
                if let Some(sub) = sub {
                    if sub.contains('/') || sub.contains('\\') || sub == ".." {
                        return Err(format!("subdirektori tidak valid: {sub}"));
                    }
                    path = path.join(sub);
                }
                Ok(Self::make_file(heap, path.to_string_lossy().into_owned()))
            })()),
            ("openFileInput", "(Ljava/lang/String;)Ljava/io/FileInputStream;") => Some((|| {
                let name = Self::string_arg(heap, args, 1)?;
                let data = self.require_file_system()?.read(&name).map_err(|error| {
                    // Nama yang keluar dari direktori aplikasi adalah pelanggaran
                    // sandbox, bukan berkas yang kebetulan tidak ada.
                    let class = match error {
                        arunika_sandbox_manager::SandboxError::PathEscape(_) => {
                            "Ljava/lang/SecurityException;"
                        }
                        _ => "Ljava/io/FileNotFoundException;",
                    };
                    NativeException::new(class, format!("{name}: {error}")).encode()
                })?;
                Ok(heap.alloc(Object::Stream {
                    type_name: "Ljava/io/FileInputStream;".to_string(),
                    name,
                    data,
                    position: 0,
                }))
            })(
            )),
            ("openFileOutput", "(Ljava/lang/String;I)Ljava/io/FileOutputStream;") => {
                Some((|| {
                    let name = Self::string_arg(heap, args, 1)?;
                    let mode = Self::int_arg(args, 2)?;
                    let file_system = self.require_file_system()?;
                    // MODE_APPEND (0x8000) melanjutkan isi lama; mode lain
                    // memotong berkas seperti MODE_PRIVATE.
                    // MODE_APPEND pada berkas baru bukan kegagalan.
                    let data = if mode & 0x8000 != 0 {
                        file_system.read(&name).unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let position = data.len();
                    Ok(heap.alloc(Object::Stream {
                        type_name: "Ljava/io/FileOutputStream;".to_string(),
                        name,
                        data,
                        position,
                    }))
                })())
            }
            ("deleteFile", "(Ljava/lang/String;)Z") => Some((|| {
                let name = Self::string_arg(heap, args, 1)?;
                match self.require_file_system()?.remove(&name) {
                    Ok(()) => Ok(Value::Int(1)),
                    Err(_) => Ok(Value::Int(0)),
                }
            })()),
            ("fileList", "()[Ljava/lang/String;") => Some((|| {
                let names = self
                    .require_file_system()?
                    .list()
                    .map_err(|error| error.to_string())?;
                let mut elements = Vec::with_capacity(names.len());
                for name in names {
                    elements.push(heap.alloc(Object::String(name)));
                }
                Ok(heap.alloc(Object::Array {
                    element_type: "Ljava/lang/String;".to_string(),
                    elements,
                }))
            })()),
            // Konfigurasi jendela Activity diterima sebagai no-op karena runtime
            // belum membuat jendela; tidak ada klaim bahwa tampilan muncul.
            ("setTheme", "(I)V")
            | ("setTheme", "(Landroid/content/res/Resources$Theme;)V")
            | ("setContentView", "(I)V")
            | ("setContentView", "(Landroid/view/View;)V")
            | ("setContentView", "(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V")
            | ("setRequestedOrientation", "(I)V")
            | ("setTitle", "(Ljava/lang/CharSequence;)V")
            | ("setTitle", "(I)V")
            | ("finish", "()V")
            | ("addContentView", "(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V") => {
                // setContentView membuat root ViewGroup dalam hierarki view
                // dan sebuah Bitmap offscreen sebagai target render kanvas.
                let view_ref = args.get(1).and_then(|value| value.as_ref());
                let mut window = self.window.lock().expect("kunci jendela");
                if window.width == 0 {
                    window.width = VIRTUAL_WIDTH as u32;
                }
                if window.height == 0 {
                    window.height = VIRTUAL_HEIGHT as u32;
                }

                let root_id = if let Some(reference) = view_ref {
                    self.ensure_view_node(reference, "android.view.View")
                } else {
                    self.view_hierarchy
                        .lock()
                        .expect("kunci hierarki view")
                        .add_view(
                            View::new(1, "android.widget.FrameLayout", ViewKind::ViewGroup)
                                .with_layout_params(LayoutParams::match_parent()),
                            None,
                        )
                        .unwrap_or(1)
                };
                {
                    let mut hierarchy = self.view_hierarchy.lock().expect("kunci hierarki view");
                    let _ = hierarchy.set_root(root_id);
                }

                let window_width = window.width.max(1);
                let window_height = window.height.max(1);
                let bitmap_result = self
                    .graphics_engine
                    .lock()
                    .expect("kunci engine grafis")
                    .buat_bitmap(window_width, window_height);
                let bitmap_created = bitmap_result.is_ok();
                let bitmap_id = bitmap_result.unwrap_or(0);

                let view_count = self
                    .view_hierarchy
                    .lock()
                    .expect("kunci hierarki view")
                    .len();
                let bounds = self
                    .view_hierarchy
                    .lock()
                    .expect("kunci hierarki view")
                    .layout(window_width as i32, window_height as i32);

                if bitmap_created {
                    let mut graphics = self.graphics_engine.lock().expect("kunci engine grafis");
                    for (index, bound) in bounds.iter().enumerate() {
                        if bound.width <= 0 || bound.height <= 0 {
                            continue;
                        }
                        let color = arunika_graphics_engine::Color::from_argb(
                            0xFF336699u32.wrapping_add((index as u32).wrapping_mul(0x00112233)),
                        );
                        let _ = graphics.draw_rect(
                            bitmap_id,
                            bound.x,
                            bound.y,
                            bound.x + bound.width,
                            bound.y + bound.height,
                            color,
                        );
                    }
                }

                window.present = true;
                window.view_count = view_count;
                window.draw_calls = bounds.len() as u64;
                Some(Ok(Value::Int(0)))
            }
            ("requestWindowFeature", "(I)Z") => Some(Ok(Value::Int(1))),
            ("isFinishing", "()Z") | ("isDestroyed", "()Z") => Some(Ok(Value::Int(0))),
            ("getTitle", "()Ljava/lang/CharSequence;") => {
                Some(Ok(heap.alloc(Object::String(String::new()))))
            }
            ("findViewById", "(I)Landroid/view/View;") => Some(Ok(Value::Int(0))),
            ("getWindow", "()Landroid/view/Window;") => {
                let mut state = self.window.lock().expect("kunci jendela");
                state.present = true;
                Some(Ok(Self::framework_instance(heap, "Landroid/view/Window;")))
            }
            ("getIntent", "()Landroid/content/Intent;") => Some(Ok(Value::Int(0))),
            // Satu thread: Runnable dijalankan pada drain Handler setelah
            // lifecycle, sama seperti post ke main looper.
            ("runOnUiThread", "(Ljava/lang/Runnable;)V") => Some((|| {
                let task = args.get(1).copied().ok_or("runOnUiThread butuh Runnable")?;
                let handler = Self::framework_instance(heap, "Landroid/os/Handler;");
                let reference = handler.as_ref().ok_or("Handler tidak teralokasi")?;
                Self::reset_queue(heap, "Landroid/os/Handler;", reference)?;
                Self::push_to_queue(heap, "Landroid/os/Handler;", reference, task)?;
                Ok(Value::Int(0))
            })()),
            ("<init>", "()V")
            | ("onCreate", "(Landroid/os/Bundle;)V")
            | ("onStart", "()V")
            | ("onResume", "()V")
            | ("onPause", "()V")
            | ("onStop", "()V")
            | ("onDestroy", "()V")
            | ("onRestart", "()V") => Some(Ok(Value::Int(0))),
            _ => None,
        }
    }

    /// `WindowManager` minimal: hanya memberi objek Display virtual.
    fn window_manager_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getDefaultDisplay", "()Landroid/view/Display;") => {
                Some(Ok(Self::framework_instance(heap, "Landroid/view/Display;")))
            }
            _ => None,
        }
    }

    /// `Display` virtual: ukuran dan kerapatan tetap sampai ada jendela nyata.
    fn display_method(method: &str, signature: &str) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getWidth", "()I") => Some(Ok(Value::Int(VIRTUAL_WIDTH))),
            ("getHeight", "()I") => Some(Ok(Value::Int(VIRTUAL_HEIGHT))),
            ("getRotation", "()I") => Some(Ok(Value::Int(0))),
            ("getRefreshRate", "()F") => Some(Ok(Value::from_float(60.0))),
            ("getMetrics", "(Landroid/util/DisplayMetrics;)V")
            | ("getRealMetrics", "(Landroid/util/DisplayMetrics;)V") => Some(Ok(Value::Int(0))),
            _ => None,
        }
    }

    /// `Resources` minimal: hanya konfigurasi tampilan virtual.
    ///
    /// Tabel resource APK belum diurai menjadi nilai, jadi `getString`/
    /// `getIdentifier` melaporkan tidak ditemukan alih-alih mengarang teks.
    /// `android.content.res.AssetManager`: aset read-only dari dalam APK.
    ///
    /// `open` mengembalikan aliran memori yang dibaca seperti `InputStream`
    /// biasa. Aset yang tidak ada dilaporkan sebagai `FileNotFoundException`
    /// agar kode aplikasi melihat kegagalan sebenarnya, bukan aliran kosong.
    fn asset_manager_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("open", "(Ljava/lang/String;)Ljava/io/InputStream;")
            | ("open", "(Ljava/lang/String;I)Ljava/io/InputStream;") => Some((|| {
                let name = Self::string_arg(heap, args, 1)?;
                let assets = self.require_assets()?;
                let data = assets
                    .read(&name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| {
                        NativeException::new(
                            "java.io.FileNotFoundException",
                            format!("aset '{name}' tidak ada di APK"),
                        )
                        .encode()
                    })?;
                Ok(heap.alloc(Object::Stream {
                    type_name: "Ljava/io/InputStream;".to_string(),
                    name,
                    data,
                    position: 0,
                }))
            })()),
            ("list", "(Ljava/lang/String;)[Ljava/lang/String;") => Some((|| {
                let prefix = Self::string_arg(heap, args, 1)?;
                let assets = self.require_assets()?;
                let names = assets.list(&prefix).map_err(|error| error.to_string())?;
                let elements = names
                    .into_iter()
                    .map(|name| heap.alloc(Object::String(name)))
                    .collect();
                Ok(heap.alloc(Object::Array {
                    element_type: "Ljava/lang/String;".to_string(),
                    elements,
                }))
            })()),
            ("getLocales", "()[Ljava/lang/String;") => Some(Ok(heap.alloc(Object::Array {
                element_type: "Ljava/lang/String;".to_string(),
                elements: Vec::new(),
            }))),
            ("close", "()V") => Some(Ok(Value::Int(0))),
            ("openFd", _) => Some(Err(
                "AssetManager.openFd belum tersedia: runtime tidak membuka deskriptor berkas APK"
                    .to_string(),
            )),
            _ => None,
        }
    }

    fn fill_bitmap(heap: &mut Heap, bitmap: Option<u32>, color: i32) {
        if let Some(index) = bitmap
            && let Some(Object::Bitmap { pixels, .. }) = heap.get_mut(index)
        {
            let rgba = [
                ((color >> 16) & 0xFF) as u8,
                ((color >> 8) & 0xFF) as u8,
                (color & 0xFF) as u8,
                0xFF,
            ];
            for chunk in pixels.chunks_mut(4) {
                chunk.copy_from_slice(&rgba);
            }
        }
    }

    fn fill_rect(
        heap: &mut Heap,
        bitmap: Option<u32>,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        color: i32,
    ) {
        if let Some(index) = bitmap
            && let Some(Object::Bitmap {
                width,
                height,
                pixels,
            }) = heap.get_mut(index)
        {
            let (w, h) = (*width as i32, *height as i32);
            let l = left.max(0).min(w);
            let t = top.max(0).min(h);
            let r = right.max(0).min(w);
            let b = bottom.max(0).min(h);
            if l >= r || t >= b {
                return;
            }
            let rgba = [
                ((color >> 16) & 0xFF) as u8,
                ((color >> 8) & 0xFF) as u8,
                (color & 0xFF) as u8,
                0xFF,
            ];
            for y in t..b {
                for x in l..r {
                    let start = ((y * w + x) * 4) as usize;
                    if start + 4 <= pixels.len() {
                        pixels[start..start + 4].copy_from_slice(&rgba);
                    }
                }
            }
        }
    }

    fn record_draw<F: FnOnce(&mut Heap, Option<u32>)>(
        &self,
        heap: &mut Heap,
        canvas_ref: Option<u32>,
        op: F,
    ) {
        let bitmap = canvas_ref.and_then(|canvas| match heap.get(canvas) {
            Some(Object::Instance { fields, .. }) => {
                fields.get(&0).and_then(|value| value.as_ref())
            }
            _ => None,
        });
        op(heap, bitmap);
        // Dimensi jendela offscreen mengikuti bitmap yang benar-benar
        // digambar bila belum pernah ditetapkan, agar laporan tidak
        // menyatakan present dengan ukuran 0x0.
        let bitmap_size = bitmap.and_then(|bitmap_id| {
            self.graphics_engine
                .lock()
                .expect("kunci engine grafis")
                .bitmap(bitmap_id as u64)
                .map(|bitmap| (bitmap.lebar, bitmap.tinggi))
        });
        let mut state = self.window.lock().expect("kunci jendela");
        if let Some((width, height)) = bitmap_size
            && width > 0
            && height > 0
            && (state.width == 0 || state.height == 0)
        {
            state.width = width;
            state.height = height;
        }
        state.draw_calls += 1;
        state.present = true;
    }

    fn bump_draw(&self, heap: &mut Heap, canvas_ref: Option<u32>) {
        let bitmap = canvas_ref.and_then(|canvas| match heap.get(canvas) {
            Some(Object::Instance { fields, .. }) => {
                fields.get(&0).and_then(|value| value.as_ref())
            }
            _ => None,
        });
        let bitmap_size = bitmap.and_then(|bitmap_id| {
            self.graphics_engine
                .lock()
                .expect("kunci engine grafis")
                .bitmap(bitmap_id as u64)
                .map(|bitmap| (bitmap.lebar, bitmap.tinggi))
        });
        let mut state = self.window.lock().expect("kunci jendela");
        if let Some((width, height)) = bitmap_size
            && width > 0
            && height > 0
            && (state.width == 0 || state.height == 0)
        {
            state.width = width;
            state.height = height;
        }
        state.draw_calls += 1;
        state.present = true;
    }

    fn window_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
        _args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getDecorView", "()Landroid/view/View;") => {
                Some(Ok(Self::framework_instance(heap, "Landroid/view/View;")))
            }
            ("setContentView", "(I)V")
            | ("setContentView", "(Landroid/view/View;)V")
            | ("setContentView", "(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V")
            | ("addContentView", "(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V") => {
                let mut state = self.window.lock().expect("kunci jendela");
                state.present = true;
                if state.width == 0 {
                    state.width = VIRTUAL_WIDTH as u32;
                }
                if state.height == 0 {
                    state.height = VIRTUAL_HEIGHT as u32;
                }
                Some(Ok(Value::Int(0)))
            }
            ("findViewById", "(I)Landroid/view/View;") => {
                Some(Ok(Self::framework_instance(heap, "Landroid/view/View;")))
            }
            ("setTitle", "(Ljava/lang/CharSequence;)V") | ("setTitle", "(I)V") => {
                Some(Ok(Value::Int(0)))
            }
            ("setFlags", "(II)V") | ("addFlags", "(I)V") | ("clearFlags", "(I)V") => {
                Some(Ok(Value::Int(0)))
            }
            ("getAttributes", "()Landroid/view/WindowManager$LayoutParams;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/view/WindowManager$LayoutParams;"),
            )),
            ("setBackgroundDrawable", "(Landroid/graphics/drawable/Drawable;)V") => {
                Some(Ok(Value::Int(0)))
            }
            _ => None,
        }
    }

    fn view_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        let this_ref = args.first().and_then(|value| value.as_ref());
        match (method, signature) {
            ("findViewById", "(I)Landroid/view/View;") => {
                let id = Self::int_arg(args, 1).unwrap_or(0);
                // Cari node dengan id yang sama; bila tidak ada, buat instance baru
                // yang dipetakan ke id tersebut agar `getId()` konsisten.
                let instance = Self::framework_instance(heap, "Landroid/view/View;");
                if let Some(reference) = instance.as_ref() {
                    self.view_nodes
                        .lock()
                        .expect("kunci view")
                        .insert(reference, id as u32);
                }
                Some(Ok(instance))
            }
            ("getId", "()I") => Some(Ok(Value::Int(
                this_ref
                    .map(|reference| self.view_id_for(reference))
                    .unwrap_or(0),
            ))),
            ("setId", "(I)V") => {
                if let Some(reference) = this_ref {
                    self.view_android_ids
                        .lock()
                        .expect("kunci id view")
                        .insert(reference, Self::int_arg(args, 1).unwrap_or(0));
                }
                Some(Ok(Value::Int(0)))
            }
            ("getVisibility", "()I") => Some(Ok(Value::Int(0))),
            ("setVisibility", "(I)V")
            | ("setLayoutParams", "(Landroid/view/ViewGroup$LayoutParams;)V")
            | ("setOnClickListener", "(Landroid/view/View$OnClickListener;)V")
            | ("setEnabled", "(Z)V")
            | ("invalidate", "()V")
            | ("requestLayout", "()V") => Some(Ok(Value::Int(0))),
            _ => None,
        }
    }

    fn view_group_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        let parent_ref = args.first().and_then(|value| value.as_ref()).unwrap_or(0);
        match (method, signature) {
            ("addView", "(Landroid/view/View;)V")
            | ("addView", "(Landroid/view/View;I)V")
            | ("addView", "(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V") => {
                let child_ref = args.get(1).and_then(|value| value.as_ref());
                if let Some(child_ref) = child_ref {
                    let child_class = heap
                        .get(child_ref)
                        .map(|object| match object {
                            Object::Instance { type_name, .. } => type_name.clone(),
                            _ => "Landroid/view/View;".to_string(),
                        })
                        .unwrap_or_else(|| "Landroid/view/View;".to_string());
                    self.add_view_child(parent_ref, child_ref, &child_class);
                }
                Some(Ok(Value::Int(0)))
            }
            ("removeView", "(Landroid/view/View;)V")
            | ("removeViewAt", "(I)V")
            | ("removeAllViews", "()V") => Some(Ok(Value::Int(0))),
            ("getChildCount", "()I") => {
                Some(Ok(Value::Int(self.view_child_count(parent_ref) as i32)))
            }
            ("getChildAt", "(I)Landroid/view/View;") => {
                let index = Self::int_arg(args, 1).unwrap_or(0);
                match self.view_child_at(heap, parent_ref, index) {
                    Some(value) => Some(Ok(value)),
                    None => Some(Ok(Value::Int(0))),
                }
            }
            _ => self.view_method(heap, method, signature, args),
        }
    }

    fn ensure_view_node(&self, reference: u32, class_name: &str) -> u32 {
        {
            let ids = self.view_nodes.lock().expect("kunci view");
            if let Some(id) = ids.get(&reference) {
                return *id;
            }
        }
        let kind = if class_name.contains("ViewGroup")
            || class_name.contains("FrameLayout")
            || class_name.contains("LinearLayout")
            || class_name.contains("RelativeLayout")
        {
            ViewKind::ViewGroup
        } else {
            ViewKind::View
        };
        let id = self
            .view_hierarchy
            .lock()
            .expect("kunci hierarki view")
            .add_view(View::new(0, class_name, kind), None)
            .unwrap_or(0);
        self.view_nodes
            .lock()
            .expect("kunci view")
            .insert(reference, id);
        id
    }

    fn add_view_child(&self, parent_ref: u32, child_ref: u32, child_class: &str) {
        let parent_id = self.ensure_view_node(parent_ref, "android.view.ViewGroup");
        let mut ids = self.view_nodes.lock().expect("kunci view");
        if ids.contains_key(&child_ref) {
            return;
        }
        let id = self
            .view_hierarchy
            .lock()
            .expect("kunci hierarki view")
            .add_view(View::new(0, child_class, ViewKind::View), Some(parent_id))
            .unwrap_or(0);
        ids.insert(child_ref, id);
    }

    fn view_child_count(&self, parent_ref: u32) -> usize {
        let parent_id = match self
            .view_nodes
            .lock()
            .expect("kunci view")
            .get(&parent_ref)
            .copied()
        {
            Some(id) => id,
            None => return 0,
        };
        self.view_hierarchy
            .lock()
            .expect("kunci hierarki view")
            .view_by_id(parent_id)
            .map(|view| view.children.len())
            .unwrap_or(0)
    }

    fn view_child_at(&self, heap: &mut Heap, parent_ref: u32, index: i32) -> Option<Value> {
        let parent_id = self
            .view_nodes
            .lock()
            .expect("kunci view")
            .get(&parent_ref)
            .copied()?;
        let child_id = self
            .view_hierarchy
            .lock()
            .expect("kunci hierarki view")
            .view_by_id(parent_id)?
            .children
            .get(index as usize)
            .copied()?;
        let instance = Self::framework_instance(heap, "Landroid/view/View;");
        if let Some(reference) = instance.as_ref() {
            self.view_nodes
                .lock()
                .expect("kunci view")
                .insert(reference, child_id);
        }
        Some(instance)
    }

    fn view_id_for(&self, reference: u32) -> i32 {
        if let Some(id) = self
            .view_android_ids
            .lock()
            .expect("kunci id view")
            .get(&reference)
            .copied()
        {
            return id;
        }
        self.view_nodes
            .lock()
            .expect("kunci view")
            .get(&reference)
            .copied()
            .unwrap_or(0) as i32
    }

    fn canvas_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        let canvas_ref = args.first().and_then(|value| value.as_ref());
        match (method, signature) {
            ("<init>", "(Landroid/graphics/Bitmap;)V") => {
                if let (Some(canvas), Some(bitmap)) =
                    (canvas_ref, args.get(1).and_then(|value| value.as_ref()))
                    && let Some(Object::Instance { fields, .. }) = heap.get_mut(canvas)
                {
                    fields.insert(0, Value::Ref(bitmap));
                }
                Some(Ok(Value::Int(0)))
            }
            ("drawColor", "(I)V") => Some((|| {
                let color = Self::int_arg(args, 1)?;
                self.record_draw(heap, canvas_ref, |heap, bitmap| {
                    Self::fill_bitmap(heap, bitmap, color);
                });
                Ok(Value::Int(0))
            })()),
            ("drawRGB", "(III)V") => Some((|| {
                let r = Self::int_arg(args, 1)?;
                let g = Self::int_arg(args, 2)?;
                let b = Self::int_arg(args, 3)?;
                let color = (0xFF << 24) | ((r & 0xFF) << 16) | ((g & 0xFF) << 8) | (b & 0xFF);
                self.record_draw(heap, canvas_ref, |heap, bitmap| {
                    Self::fill_bitmap(heap, bitmap, color);
                });
                Ok(Value::Int(0))
            })()),
            ("drawRect", "(FFFFI)V") => Some((|| {
                let left = Self::int_arg(args, 1)?;
                let top = Self::int_arg(args, 2)?;
                let right = Self::int_arg(args, 3)?;
                let bottom = Self::int_arg(args, 4)?;
                let color = Self::int_arg(args, 5)?;
                self.record_draw(heap, canvas_ref, |heap, bitmap| {
                    Self::fill_rect(heap, bitmap, left, top, right, bottom, color);
                });
                Ok(Value::Int(0))
            })()),
            ("drawLine", "(FFFFI)V") => Some((|| {
                let _ = (
                    Self::int_arg(args, 1)?,
                    Self::int_arg(args, 2)?,
                    Self::int_arg(args, 3)?,
                    Self::int_arg(args, 4)?,
                    Self::int_arg(args, 5)?,
                );
                self.bump_draw(heap, canvas_ref);
                Ok(Value::Int(0))
            })()),
            ("drawText", "(Ljava/lang/String;FF)V")
            | ("drawText", "(Ljava/lang/String;FFLandroid/graphics/Paint;)V") => Some((|| {
                let _ = (
                    Self::string_arg(heap, args, 1),
                    Self::int_arg(args, 2)?,
                    Self::int_arg(args, 3)?,
                );
                self.bump_draw(heap, canvas_ref);
                Ok(Value::Int(0))
            })(
            )),
            ("drawBitmap", "(Landroid/graphics/Bitmap;FFLandroid/graphics/Paint;)V") => {
                self.bump_draw(heap, canvas_ref);
                Some(Ok(Value::Int(0)))
            }
            _ => None,
        }
    }

    fn bitmap_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("createBitmap", "(IILandroid/graphics/Bitmap$Config;)Landroid/graphics/Bitmap;") => {
                Some((|| {
                    let width = Self::int_arg(args, 0)?;
                    let height = Self::int_arg(args, 1)?;
                    if width <= 0 || height <= 0 || width > 8192 || height > 8192 {
                        return Err("Bitmap.createBitmap: ukuran tidak valid".to_string());
                    }
                    let pixels = vec![0u8; width as usize * height as usize * 4];
                    Ok(heap.alloc(Object::Bitmap {
                        width: width as u32,
                        height: height as u32,
                        pixels,
                    }))
                })())
            }
            ("getWidth", "()I") => {
                let width = args
                    .first()
                    .and_then(|value| value.as_ref())
                    .and_then(|index| match heap.get(index) {
                        Some(Object::Bitmap { width, .. }) => Some(*width as i32),
                        _ => None,
                    });
                Some(
                    width
                        .map(Value::Int)
                        .ok_or_else(|| "bukan Bitmap".to_string()),
                )
            }
            ("getHeight", "()I") => {
                let height = args
                    .first()
                    .and_then(|value| value.as_ref())
                    .and_then(|index| match heap.get(index) {
                        Some(Object::Bitmap { height, .. }) => Some(*height as i32),
                        _ => None,
                    });
                Some(
                    height
                        .map(Value::Int)
                        .ok_or_else(|| "bukan Bitmap".to_string()),
                )
            }
            ("recycle", "()V") | ("isRecycled", "()Z") => Some(Ok(Value::Int(0))),
            ("getConfig", "()Landroid/graphics/Bitmap$Config;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/graphics/Bitmap$Config;"),
            )),
            _ => None,
        }
    }

    /// Indeks field pada instance `KeyEvent`/`InputEvent`.
    const KEY_FIELD_ACTION: u32 = 0;
    const KEY_FIELD_CODE: u32 = 1;
    const KEY_FIELD_REPEAT: u32 = 2;
    const KEY_FIELD_DOWN_TIME: u32 = 3;
    const KEY_FIELD_EVENT_TIME: u32 = 4;
    const KEY_FIELD_META: u32 = 5;
    const KEY_FIELD_SOURCE: u32 = 6;

    /// `android.view.InputEvent` dan `android.view.KeyEvent`.
    ///
    /// Runtime ini belum memiliki jendela host, sehingga event dimasukkan
    /// secara terprogram (lihat `new_key_event`) dan diteruskan ke activity
    /// lewat `dispatchKeyEvent`/`onKeyDown`/`onKeyUp` oleh CLI. Konstruktor
    /// dan getter disediakan agar aplikasi dapat membaca event yang diterima.
    fn key_event_method(
        &self,
        heap: &mut Heap,
        class_descriptor: &str,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (class_descriptor, method, signature) {
            ("Landroid/view/KeyEvent;", "<init>", "(II)V") => Some((|| {
                let action = Self::int_arg(args, 1)?;
                let key_code = Self::int_arg(args, 2)?;
                Self::store_key_event(heap, Self::this_ref(args)?, action, key_code, 0, 0, 0, 0, 0)
            })()),
            ("Landroid/view/KeyEvent;", "<init>", "(JJIIII)V") => Some((|| {
                let down_time = Self::long_arg(args, 1)?;
                let event_time = Self::long_arg(args, 3)?;
                let action = Self::int_arg(args, 5)?;
                let key_code = Self::int_arg(args, 6)?;
                let repeat = Self::int_arg(args, 7)?;
                let meta = Self::int_arg(args, 8)?;
                Self::store_key_event(
                    heap,
                    Self::this_ref(args)?,
                    action,
                    key_code,
                    repeat,
                    down_time,
                    event_time,
                    meta,
                    0,
                )
            })()),
            ("Landroid/view/KeyEvent;", "getAction", "()I") => Some(Ok(Self::key_event_field(
                heap,
                args,
                Self::KEY_FIELD_ACTION,
            ))),
            ("Landroid/view/KeyEvent;", "getKeyCode", "()I") => {
                Some(Ok(Self::key_event_field(heap, args, Self::KEY_FIELD_CODE)))
            }
            ("Landroid/view/KeyEvent;", "getRepeatCount", "()I") => Some(Ok(
                Self::key_event_field(heap, args, Self::KEY_FIELD_REPEAT),
            )),
            ("Landroid/view/KeyEvent;", "getDownTime", "()J") => Some(Ok(Self::key_event_field(
                heap,
                args,
                Self::KEY_FIELD_DOWN_TIME,
            ))),
            ("Landroid/view/KeyEvent;", "getEventTime", "()J") => Some(Ok(Self::key_event_field(
                heap,
                args,
                Self::KEY_FIELD_EVENT_TIME,
            ))),
            ("Landroid/view/KeyEvent;", "getMetaState", "()I") => {
                Some(Ok(Self::key_event_field(heap, args, Self::KEY_FIELD_META)))
            }
            ("Landroid/view/KeyEvent;", "getSource", "()I") => Some(Ok(Self::key_event_field(
                heap,
                args,
                Self::KEY_FIELD_SOURCE,
            ))),
            ("Landroid/view/KeyEvent;", "isDown", "()Z") => {
                let action = match Self::key_event_field(heap, args, Self::KEY_FIELD_ACTION) {
                    Value::Int(value) => value,
                    _ => 0,
                };
                Some(Ok(Value::Int(if action == 0 { 1 } else { 0 })))
            }
            ("Landroid/view/InputEvent;", "getEventTime", "()J") => Some(Ok(
                Self::key_event_field(heap, args, Self::KEY_FIELD_EVENT_TIME),
            )),
            ("Landroid/view/InputEvent;", "getDownTime", "()J") => Some(Ok(Self::key_event_field(
                heap,
                args,
                Self::KEY_FIELD_DOWN_TIME,
            ))),
            ("Landroid/view/InputEvent;", "getSource", "()I") => Some(Ok(Self::key_event_field(
                heap,
                args,
                Self::KEY_FIELD_SOURCE,
            ))),
            _ => None,
        }
    }

    /// Baca field instance `KeyEvent`/`InputEvent` lewat indeksnya.
    fn key_event_field(heap: &Heap, args: &[Value], index: u32) -> Value {
        match Self::this_ref(args)
            .ok()
            .and_then(|reference| heap.get(reference))
        {
            Some(Object::Instance { fields, .. }) => {
                fields.get(&index).copied().unwrap_or_else(|| match index {
                    Self::KEY_FIELD_DOWN_TIME | Self::KEY_FIELD_EVENT_TIME => Value::from_long(0),
                    _ => Value::Int(0),
                })
            }
            _ => Value::Int(0),
        }
    }

    /// Tulis seluruh field `KeyEvent` ke instance yang sudah dialokasikan.
    #[allow(clippy::too_many_arguments)]
    fn store_key_event(
        heap: &mut Heap,
        reference: u32,
        action: i32,
        key_code: i32,
        repeat: i32,
        down_time: i64,
        event_time: i64,
        meta_state: i32,
        source: i32,
    ) -> Result<Value, String> {
        match heap.get_mut(reference) {
            Some(Object::Instance { fields, .. }) => {
                fields.insert(Self::KEY_FIELD_ACTION, Value::Int(action));
                fields.insert(Self::KEY_FIELD_CODE, Value::Int(key_code));
                fields.insert(Self::KEY_FIELD_REPEAT, Value::Int(repeat));
                fields.insert(Self::KEY_FIELD_DOWN_TIME, Value::from_long(down_time));
                fields.insert(Self::KEY_FIELD_EVENT_TIME, Value::from_long(event_time));
                fields.insert(Self::KEY_FIELD_META, Value::Int(meta_state));
                fields.insert(Self::KEY_FIELD_SOURCE, Value::Int(source));
                Ok(Value::Int(0))
            }
            _ => Err("objek KeyEvent tidak ada di heap".to_string()),
        }
    }

    /// Alokasikan `KeyEvent` dengan aksi dan kode tombol tertentu. Dipakai
    /// oleh CLI untuk memasukkan event keyboard ke activity secara
    /// terprogram; field indeks tetap konsisten dengan `key_event_method`.
    pub fn new_key_event(&self, heap: &mut Heap, action: i32, key_code: i32) -> Value {
        let reference = heap.alloc(Object::Instance {
            type_name: "Landroid/view/KeyEvent;".to_string(),
            fields: std::collections::HashMap::new(),
        });
        if let Value::Ref(index) = reference {
            let _ = Self::store_key_event(heap, index, action, key_code, 0, 0, 0, 0, 0);
        }
        reference
    }

    fn resources_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getDisplayMetrics", "()Landroid/util/DisplayMetrics;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/util/DisplayMetrics;"),
            )),
            ("getConfiguration", "()Landroid/content/res/Configuration;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/content/res/Configuration;"),
            )),
            ("getIdentifier", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I") => {
                let name = Self::string_arg(heap, args, 1).ok();
                let def_type = Self::string_arg(heap, args, 2).ok();
                let id = self.resolve_resource(name.as_deref(), def_type.as_deref());
                Some(Ok(Value::Int(id.unwrap_or(0))))
            }
            ("openRawResource", "(I)Ljava/io/InputStream;") => Some((|| {
                let id = Self::int_arg(args, 1)?;
                let data = self
                    .read_resource(id)
                    .ok_or_else(|| "resource tidak ada di APK".to_string())?;
                Ok(heap.alloc(Object::Stream {
                    type_name: "Ljava/io/InputStream;".to_string(),
                    name: format!("res/{id}"),
                    data,
                    position: 0,
                }))
            })()),
            ("getResourceTypeName", "(I)Ljava/lang/String;") => Some((|| {
                let id = Self::int_arg(args, 1)?;
                let names = self
                    .resources
                    .as_ref()
                    .ok_or_else(|| "runtime tidak diberi akses resource APK".to_string())?
                    .entries()
                    .map_err(|error| error.to_string())?;
                let index = usize::try_from(id)
                    .ok()
                    .and_then(|value| value.checked_sub(1))
                    .ok_or_else(|| "id resource tidak valid".to_string())?;
                let entry = names
                    .get(index)
                    .ok_or_else(|| "id resource tidak valid".to_string())?;
                let folder = entry.split('/').next().unwrap_or("raw");
                let type_name = folder.split('-').next().unwrap_or(folder).to_string();
                Ok(heap.alloc(Object::String(type_name)))
            })()),
            ("getResourceEntryName", "(I)Ljava/lang/String;") => Some((|| {
                let id = Self::int_arg(args, 1)?;
                let names = self
                    .resources
                    .as_ref()
                    .ok_or_else(|| "runtime tidak diberi akses resource APK".to_string())?
                    .entries()
                    .map_err(|error| error.to_string())?;
                let index = usize::try_from(id)
                    .ok()
                    .and_then(|value| value.checked_sub(1))
                    .ok_or_else(|| "id resource tidak valid".to_string())?;
                let entry = names
                    .get(index)
                    .ok_or_else(|| "id resource tidak valid".to_string())?;
                let last = entry.split('/').next_back().unwrap_or(entry);
                let entry_name = last
                    .rsplit_once('.')
                    .map(|(left, _)| left)
                    .unwrap_or(last)
                    .to_string();
                Ok(heap.alloc(Object::String(entry_name)))
            })()),
            ("getColor", "(I)I") => Some(Ok(Value::Int(0))),
            ("getBoolean", "(I)Z") => Some(Ok(Value::Int(0))),
            ("getInteger", "(I)I") => Some(Ok(Value::Int(0))),
            ("getDimensionPixelSize", "(I)I") | ("getDimensionPixelOffset", "(I)I") => {
                Some(Ok(Value::Int(0)))
            }
            ("getString", "(I)Ljava/lang/String;") | ("getText", "(I)Ljava/lang/CharSequence;") => {
                Some((|| {
                    let id = Self::int_arg(args, 1)?;
                    let data = self
                        .read_resource(id)
                        .ok_or_else(|| "resource tidak ada di APK".to_string())?;
                    let text = String::from_utf8_lossy(&data).into_owned();
                    Ok(heap.alloc(Object::String(text)))
                })())
            }
            _ => None,
        }
    }

    /// `WeakReference`: referen disimpan pada field 0 tanpa pengumpulan sampah.
    ///
    /// Heap runtime tidak punya GC, jadi referen selalu hidup selama engine
    /// hidup. Perilaku ini dilaporkan apa adanya, bukan disamarkan.
    fn weak_reference_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "(Ljava/lang/Object;)V")
            | ("<init>", "(Ljava/lang/Object;Ljava/lang/ref/ReferenceQueue;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let target = args.get(1).copied().unwrap_or(Value::Int(0));
                match heap.get_mut(reference) {
                    Some(Object::Instance { fields, .. }) => {
                        fields.insert(0, target);
                        Ok(Value::Int(0))
                    }
                    _ => Err("objek referensi tidak ada di heap".to_string()),
                }
            })(
            )),
            ("get", "()Ljava/lang/Object;") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get(reference) {
                    Some(Object::Instance { fields, .. }) => {
                        Ok(fields.get(&0).copied().unwrap_or(Value::Int(0)))
                    }
                    _ => Err("objek referensi tidak ada di heap".to_string()),
                }
            })()),
            ("clear", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get_mut(reference) {
                    Some(Object::Instance { fields, .. }) => {
                        fields.insert(0, Value::Int(0));
                        Ok(Value::Int(0))
                    }
                    _ => Err("objek referensi tidak ada di heap".to_string()),
                }
            })()),
            ("enqueue", "()Z") | ("isEnqueued", "()Z") => Some(Ok(Value::Int(0))),
            _ => None,
        }
    }

    /// `android.net.LocalSocket`: soket domain Unix lokal tidak didukung.
    ///
    /// Objeknya boleh dibuat agar kode aplikasi sampai ke `connect`, lalu
    /// `connect` melempar `IOException` yang bisa ditangkap pemanggil. Runtime
    /// tidak pernah berpura-pura tersambung.
    fn local_socket_method(
        _heap: &mut Heap,
        method: &str,
        signature: &str,
        _args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "()V") | ("<init>", "(Ljava/io/FileDescriptor;)V") | ("<init>", "(I)V") => {
                Some(Ok(Value::Int(0)))
            }
            ("connect", "(Landroid/net/LocalSocketAddress;)V")
            | ("connect", "(Landroid/net/LocalSocketAddress;I)V") => {
                Some(Err(NativeException::new(
                    "Ljava/io/IOException;",
                    "LocalSocket tidak didukung runtime".to_string(),
                )
                .encode()))
            }
            ("getInputStream", "()Ljava/io/InputStream;")
            | ("getOutputStream", "()Ljava/io/OutputStream;") => Some(Err(NativeException::new(
                "Ljava/io/IOException;",
                "aliran LocalSocket tidak tersedia".to_string(),
            )
            .encode())),
            ("isConnected", "()Z") | ("isBound", "()Z") => Some(Ok(Value::Int(0))),
            ("close", "()V")
            | ("setSoTimeout", "(I)V")
            | ("setReceiveBufferSize", "(I)V")
            | ("setSendBufferSize", "(I)V") => Some(Ok(Value::Int(0))),
            ("getRemoteSocketAddress", "()Landroid/net/LocalSocketAddress;") => {
                Some(Ok(Value::Int(0)))
            }
            _ => None,
        }
    }

    /// `LocalSocketAddress` hanya menyimpan nama; tidak ada soket nyata.
    fn local_socket_address_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "(Ljava/lang/String;)V")
            | ("<init>", "(Ljava/lang/String;Landroid/net/LocalSocketAddress$Namespace;)V") => {
                Some((|| {
                    let reference = Self::this_ref(args)?;
                    let name = args.get(1).copied().unwrap_or(Value::Int(0));
                    match heap.get_mut(reference) {
                        Some(Object::Instance { fields, .. }) => {
                            fields.insert(0, name);
                            Ok(Value::Int(0))
                        }
                        _ => Err("objek LocalSocketAddress tidak ada di heap".to_string()),
                    }
                })())
            }
            ("getName", "()Ljava/lang/String;") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get(reference) {
                    Some(Object::Instance { fields, .. }) => {
                        Ok(fields.get(&0).copied().unwrap_or(Value::Int(0)))
                    }
                    _ => Err("objek LocalSocketAddress tidak ada di heap".to_string()),
                }
            })()),
            _ => None,
        }
    }

    /// `ReentrantLock` kooperatif.
    ///
    /// Eksekusi berjalan pada satu thread, jadi kunci selalu bisa diambil dan
    /// tidak ada penantian. Method yang membutuhkan penjadwalan thread
    /// dilaporkan sebagai no-op yang eksplisit.
    fn lock_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        _args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "()V") | ("<init>", "(Z)V") => Some(Ok(Value::Int(0))),
            ("lock", "()V") | ("lockInterruptibly", "()V") | ("unlock", "()V") => {
                Some(Ok(Value::Int(0)))
            }
            ("tryLock", "()Z") | ("tryLock", "(JLjava/util/concurrent/TimeUnit;)Z") => {
                Some(Ok(Value::Int(1)))
            }
            ("isLocked", "()Z") | ("isHeldByCurrentThread", "()Z") => Some(Ok(Value::Int(0))),
            ("getHoldCount", "()I") => Some(Ok(Value::Int(0))),
            ("newCondition", "()Ljava/util/concurrent/locks/Condition;") => Some(Ok(
                Self::framework_instance(heap, "Ljava/util/concurrent/locks/Condition;"),
            )),
            _ => None,
        }
    }

    /// `Condition`: menunggu dan membangunkan tidak punya arti pada satu thread.
    fn condition_method(method: &str, signature: &str) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("await", "()V")
            | ("awaitUninterruptibly", "()V")
            | ("signal", "()V")
            | ("signalAll", "()V") => Some(Ok(Value::Int(0))),
            ("await", "(JLjava/util/concurrent/TimeUnit;)Z") => Some(Ok(Value::Int(1))),
            _ => None,
        }
    }

    /// `ContentResolver` minimal: runtime tidak punya content provider.
    fn content_resolver_method(method: &str, signature: &str) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getType", "(Landroid/net/Uri;)Ljava/lang/String;")
            | (
                "query",
                "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            )
            | (
                "call",
                "(Landroid/net/Uri;Ljava/lang/String;Ljava/lang/String;Landroid/os/Bundle;)Landroid/os/Bundle;",
            )
            | (
                "acquireContentProviderClient",
                "(Landroid/net/Uri;)Landroid/content/ContentProviderClient;",
            ) => Some(Ok(Value::Int(0))),
            (
                "registerContentObserver",
                "(Landroid/net/Uri;ZLandroid/database/ContentObserver;)V",
            )
            | ("unregisterContentObserver", "(Landroid/database/ContentObserver;)V")
            | ("notifyChange", "(Landroid/net/Uri;Landroid/database/ContentObserver;)V")
            | ("notifyChange", "(Landroid/net/Uri;Landroid/database/ContentObserver;I)V") => {
                Some(Ok(Value::Int(0)))
            }
            _ => None,
        }
    }

    /// `android.provider.Settings` untuk nilai yang bisa dipalsukan dengan aman.
    ///
    /// Hanya `android_id` yang punya nilai; kunci lain dilaporkan null/default
    /// karena runtime tidak membaca penyimpanan sistem Android.
    fn settings_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            (
                "getString",
                "(Landroid/content/ContentResolver;Ljava/lang/String;)Ljava/lang/String;",
            ) => Some((|| {
                let name = Self::string_arg(heap, args, 1)?;
                if name == "android_id" {
                    Ok(heap.alloc(Object::String(self.virtual_android_id())))
                } else {
                    Ok(Value::Int(0))
                }
            })()),
            ("getInt", "(Landroid/content/ContentResolver;Ljava/lang/String;I)I")
            | ("getLong", "(Landroid/content/ContentResolver;Ljava/lang/String;J)J")
            | ("getFloat", "(Landroid/content/ContentResolver;Ljava/lang/String;F)F") => {
                Some(Ok(args.get(2).copied().unwrap_or(Value::Int(0))))
            }
            _ => None,
        }
    }

    /// ID perangkat virtual yang stabil per paket, bukan pengenal asli.
    fn virtual_android_id(&self) -> String {
        let package = self.manifest.package.as_deref().unwrap_or("arunika");
        let digest = md5::compute(package.as_bytes()).0;
        digest
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// `String.format` untuk penanda %s, %d, %f, %x, %c, %b, dan %%.
    ///
    /// Lebar, presisi, dan flag '-'/'0'/'+' didukung. Format lain dilaporkan
    /// sebagai error alih-alih menghasilkan teks yang salah.
    fn format_java(heap: &Heap, pattern: &str, args: &[Value]) -> Result<String, String> {
        let mut out = String::new();
        let mut chars = pattern.chars().peekable();
        let mut next = 0usize;
        while let Some(character) = chars.next() {
            if character != '%' {
                out.push(character);
                continue;
            }
            let mut left = false;
            let mut zero = false;
            let mut plus = false;
            while let Some(&flag) = chars.peek() {
                match flag {
                    '-' => left = true,
                    '0' => zero = true,
                    '+' => plus = true,
                    ' ' | ',' | '(' => {}
                    _ => break,
                }
                chars.next();
            }
            let mut width = 0usize;
            while let Some(&digit) = chars.peek() {
                let Some(value) = digit.to_digit(10) else {
                    break;
                };
                width = width * 10 + value as usize;
                chars.next();
            }
            let mut precision: Option<usize> = None;
            if chars.peek() == Some(&'.') {
                chars.next();
                let mut digits = 0usize;
                while let Some(&digit) = chars.peek() {
                    let Some(value) = digit.to_digit(10) else {
                        break;
                    };
                    digits = digits * 10 + value as usize;
                    chars.next();
                }
                precision = Some(digits);
            }
            let conversion = chars.next().ok_or("format tidak lengkap")?;
            if conversion == '%' {
                out.push('%');
                continue;
            }
            let argument = args.get(next).copied().unwrap_or(Value::Int(0));
            next += 1;
            let numeric = matches!(conversion, 'd' | 'f' | 'x' | 'X');
            let mut piece = match conversion {
                's' | 'S' => {
                    let mut text = Self::object_text(heap, argument);
                    if let Some(limit) = precision {
                        text.truncate(limit);
                    }
                    if conversion == 'S' {
                        text = text.to_uppercase();
                    }
                    text
                }
                'd' => {
                    let text = Self::number_text(heap, argument, 10).ok_or_else(|| {
                        format!(
                            "%d butuh bilangan bulat, diterima {argument:?} pada pola {pattern}"
                        )
                    })?;
                    let sign = if plus && !text.starts_with('-') {
                        "+"
                    } else {
                        ""
                    };
                    format!("{sign}{text}")
                }
                'f' => {
                    let number = Self::double_of(heap, argument);
                    let digits = precision.unwrap_or(6);
                    let sign = if plus && number >= 0.0 { "+" } else { "" };
                    format!("{sign}{number:.digits$}")
                }
                'x' | 'X' => {
                    let text = Self::number_text(heap, argument, 16).ok_or_else(|| {
                        format!(
                            "%x butuh bilangan bulat, diterima {argument:?} pada pola {pattern}"
                        )
                    })?;
                    if conversion == 'X' {
                        text.to_uppercase()
                    } else {
                        text
                    }
                }
                'c' => {
                    let code = Self::integer_of(heap, argument).unwrap_or(0);
                    let character = u32::try_from(code)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| format!("bukan karakter: {code}"))?;
                    character.to_string()
                }
                'b' | 'B' => {
                    let text = match argument {
                        Value::Int(0) => "false".to_string(),
                        Value::Ref(reference) if heap.get(reference).is_none() => {
                            "false".to_string()
                        }
                        _ => "true".to_string(),
                    };
                    if conversion == 'B' {
                        text.to_uppercase()
                    } else {
                        text
                    }
                }
                other => return Err(format!("penanda format belum didukung: %{other}")),
            };
            if piece.chars().count() < width {
                let pad = width - piece.chars().count();
                if left {
                    piece.push_str(&" ".repeat(pad));
                } else if zero && numeric {
                    let mut padded = String::new();
                    for character in piece.chars() {
                        if padded.is_empty() && (character == '-' || character == '+') {
                            padded.push(character);
                            continue;
                        }
                    }
                    let sign_len = padded.chars().count();
                    let zeros = pad.saturating_sub(sign_len);
                    padded.push_str(&"0".repeat(zeros));
                    padded.push_str(piece.trim_start_matches(['-', '+']));
                    piece = padded;
                } else {
                    piece = format!("{}{piece}", " ".repeat(pad));
                }
            }
            out.push_str(&piece);
        }
        Ok(out)
    }

    /// Teks bilangan pada radix tertentu; BigInteger memakai presisi penuh.
    fn number_text(heap: &Heap, value: Value, radix: u32) -> Option<String> {
        if let Value::Ref(reference) = value
            && let Some(Object::Instance { type_name, fields }) = heap.get(reference)
            && type_name == "Ljava/math/BigInteger;"
        {
            let text = fields
                .get(&0)
                .and_then(|value| value.as_ref())
                .and_then(|text| heap.string(text))?;
            let big = text.parse::<BigInt>().ok()?;
            return Some(big.to_str_radix(radix));
        }
        let number = Self::integer_of(heap, value)?;
        Some(match radix {
            10 => number.to_string(),
            16 => format!("{:x}", number as u64),
            _ => number.to_string(),
        })
    }

    fn integer_of(heap: &Heap, value: Value) -> Option<i64> {
        match value {
            Value::Int(number) => Some(i64::from(number)),
            Value::Wide(bits) => Some(bits as i64),
            Value::Ref(reference) => match heap.get(reference) {
                Some(Object::Boxed { value, .. }) => Self::integer_of(heap, *value),
                _ => None,
            },
        }
    }

    fn double_of(heap: &Heap, value: Value) -> f64 {
        match value {
            Value::Wide(bits) => f64::from_bits(bits),
            Value::Int(bits) => f64::from(f32::from_bits(bits as u32)),
            Value::Ref(reference) => match heap.get(reference) {
                Some(Object::Boxed { value, .. }) => Self::double_of(heap, *value),
                _ => 0.0,
            },
        }
    }

    /// `java.math.BigInteger` dengan presisi arbitrer.
    ///
    /// Nilai disimpan sebagai teks desimal pada field 0 sehingga heap tetap
    /// dapat diserialisasi; aritmetika memakai num-bigint.
    fn big_integer_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "(I[B)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let signum = Self::int_arg(args, 1)?;
                let magnitude = Self::byte_array_arg(heap, args, 2)?;
                let value = Self::big_from_bytes(signum, &magnitude);
                Self::set_big_integer(heap, reference, value)?;
                Ok(Value::Int(0))
            })()),
            ("<init>", "([B)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let magnitude = Self::byte_array_arg(heap, args, 1)?;
                let value = Self::big_from_bytes(1, &magnitude);
                Self::set_big_integer(heap, reference, value)?;
                Ok(Value::Int(0))
            })()),
            ("<init>", "(Ljava/lang/String;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let text = Self::string_arg(heap, args, 1)?;
                let value = text.parse::<BigInt>().map_err(|_| {
                    NativeException::new(
                        "Ljava/lang/NumberFormatException;",
                        format!("bukan bilangan: {text}"),
                    )
                    .encode()
                })?;
                Self::set_big_integer(heap, reference, value)?;
                Ok(Value::Int(0))
            })()),
            ("valueOf", "(J)Ljava/math/BigInteger;") => Some((|| {
                let value = args
                    .first()
                    .and_then(|value| value.as_long())
                    .ok_or("valueOf butuh long")?;
                Ok(Self::new_big_integer(heap, BigInt::from(value)))
            })()),
            ("add", "(Ljava/math/BigInteger;)Ljava/math/BigInteger;")
            | ("subtract", "(Ljava/math/BigInteger;)Ljava/math/BigInteger;")
            | ("multiply", "(Ljava/math/BigInteger;)Ljava/math/BigInteger;")
            | ("divide", "(Ljava/math/BigInteger;)Ljava/math/BigInteger;")
            | ("remainder", "(Ljava/math/BigInteger;)Ljava/math/BigInteger;")
            | ("mod", "(Ljava/math/BigInteger;)Ljava/math/BigInteger;")
            | ("gcd", "(Ljava/math/BigInteger;)Ljava/math/BigInteger;")
            | ("and", "(Ljava/math/BigInteger;)Ljava/math/BigInteger;")
            | ("or", "(Ljava/math/BigInteger;)Ljava/math/BigInteger;")
            | ("xor", "(Ljava/math/BigInteger;)Ljava/math/BigInteger;") => Some((|| {
                let left = Self::big_operand(heap, args, 0)?;
                let right = Self::big_operand(heap, args, 1)?;
                if matches!(method, "divide" | "remainder" | "mod") && right == BigInt::from(0) {
                    return Err(NativeException::new(
                        "Ljava/lang/ArithmeticException;",
                        "pembagi nol".to_string(),
                    )
                    .encode());
                }
                let value = match method {
                    "add" => left + right,
                    "subtract" => left - right,
                    "multiply" => left * right,
                    "divide" | "mod" => left / right,
                    "remainder" => left % right,
                    "gcd" => Self::big_gcd(left, right),
                    "and" => left & right,
                    "or" => left | right,
                    _ => left ^ right,
                };
                Ok(Self::new_big_integer(heap, value))
            })()),
            ("negate", "()Ljava/math/BigInteger;")
            | ("abs", "()Ljava/math/BigInteger;")
            | ("not", "()Ljava/math/BigInteger;") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                let result = match method {
                    "negate" => -value,
                    "abs" => value.abs(),
                    _ => !value,
                };
                Ok(Self::new_big_integer(heap, result))
            })()),
            ("pow", "(I)Ljava/math/BigInteger;") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                let exponent = Self::int_arg(args, 1)?;
                let exponent = u32::try_from(exponent).map_err(|_| {
                    NativeException::new(
                        "Ljava/lang/ArithmeticException;",
                        "eksponen negatif".to_string(),
                    )
                    .encode()
                })?;
                Ok(Self::new_big_integer(heap, value.pow(exponent)))
            })()),
            ("shiftLeft", "(I)Ljava/math/BigInteger;")
            | ("shiftRight", "(I)Ljava/math/BigInteger;") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                let amount = Self::int_arg(args, 1)?;
                let magnitude = amount.unsigned_abs();
                // Geser kiri untuk jumlah positif pada shiftLeft, dan untuk
                // jumlah negatif pada shiftRight (Java membalik arahnya).
                let to_left = (method == "shiftLeft") == (amount >= 0);
                let result = if to_left {
                    value << magnitude
                } else {
                    value >> magnitude
                };
                Ok(Self::new_big_integer(heap, result))
            })()),
            ("compareTo", "(Ljava/math/BigInteger;)I") => Some((|| {
                let left = Self::big_operand(heap, args, 0)?;
                let right = Self::big_operand(heap, args, 1)?;
                let ordering = match left.cmp(&right) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                };
                Ok(Value::Int(ordering))
            })()),
            ("equals", "(Ljava/lang/Object;)Z") => Some((|| {
                let left = Self::big_operand(heap, args, 0)?;
                let right = Self::big_operand(heap, args, 1).unwrap_or(BigInt::from(0));
                Ok(Value::Int(i32::from(left == right)))
            })()),
            ("signum", "()I") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                Ok(Value::Int(match value.sign() {
                    Sign::Minus => -1,
                    Sign::NoSign => 0,
                    Sign::Plus => 1,
                }))
            })()),
            ("bitLength", "()I") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                Ok(Value::Int(value.bits() as i32))
            })()),
            ("testBit", "(I)Z") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                let bit = Self::int_arg(args, 1)?;
                let bit = u64::try_from(bit).map_err(|_| "bit negatif".to_string())?;
                Ok(Value::Int(i32::from(value.bit(bit))))
            })()),
            ("intValue", "()I") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                Ok(Value::Int(value.to_i32().unwrap_or_else(|| {
                    let masked = value & BigInt::from(u32::MAX);
                    masked.to_i64().unwrap_or(0) as i32
                })))
            })()),
            ("longValue", "()J") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                Ok(Value::from_long(value.to_i64().unwrap_or_else(|| {
                    let masked = value & BigInt::from(u64::MAX);
                    masked.to_i64().unwrap_or(0)
                })))
            })()),
            ("doubleValue", "()D") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                Ok(Value::from_double(value.to_f64().unwrap_or(0.0)))
            })()),
            ("floatValue", "()F") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                Ok(Value::from_float(value.to_f32().unwrap_or(0.0)))
            })()),
            ("byteValue", "()B") | ("shortValue", "()S") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                let masked = value & BigInt::from(0xFFu32);
                Ok(Value::Int(masked.to_i32().unwrap_or(0)))
            })()),
            ("toString", "()Ljava/lang/String;") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                Ok(heap.alloc(Object::String(value.to_str_radix(10))))
            })()),
            ("toByteArray", "()[B") => Some((|| {
                let value = Self::big_operand(heap, args, 0)?;
                let bytes = value.to_signed_bytes_be();
                Ok(Self::byte_array(heap, &bytes))
            })()),
            _ => None,
        }
    }

    fn big_operand(heap: &Heap, args: &[Value], index: usize) -> Result<BigInt, String> {
        let reference = args
            .get(index)
            .and_then(|value| value.as_ref())
            .ok_or("argumen BigInteger harus objek")?;
        match heap.get(reference) {
            Some(Object::Instance { fields, .. }) => {
                let text = fields
                    .get(&0)
                    .and_then(|value| value.as_ref())
                    .and_then(|text| heap.string(text))
                    .ok_or("BigInteger tanpa nilai")?;
                text.parse::<BigInt>()
                    .map_err(|_| "nilai BigInteger tidak valid".to_string())
            }
            _ => Err("objek bukan BigInteger".to_string()),
        }
    }

    fn big_from_bytes(signum: i32, magnitude: &[u8]) -> BigInt {
        if signum == 0 || magnitude.is_empty() {
            return BigInt::from(0);
        }
        let sign = if signum < 0 { Sign::Minus } else { Sign::Plus };
        BigInt::from_bytes_be(sign, magnitude)
    }

    fn big_gcd(left: BigInt, right: BigInt) -> BigInt {
        let mut a = left.abs();
        let mut b = right.abs();
        while b != BigInt::from(0) {
            let remainder = &a % &b;
            a = b;
            b = remainder;
        }
        a
    }

    fn new_big_integer(heap: &mut Heap, value: BigInt) -> Value {
        let text = heap.alloc(Object::String(value.to_str_radix(10)));
        let mut fields = std::collections::HashMap::new();
        fields.insert(0, text);
        heap.alloc(Object::Instance {
            type_name: "Ljava/math/BigInteger;".to_owned(),
            fields,
        })
    }

    fn set_big_integer(heap: &mut Heap, reference: u32, value: BigInt) -> Result<(), String> {
        let text = heap.alloc(Object::String(value.to_str_radix(10)));
        match heap.get_mut(reference) {
            Some(Object::Instance { fields, .. }) => {
                fields.insert(0, text);
                Ok(())
            }
            _ => Err("objek BigInteger tidak ada di heap".to_string()),
        }
    }

    /// `java.security.MessageDigest` untuk MD5, SHA-1, SHA-256, dan SHA-512.
    ///
    /// Data yang di-`update` disimpan pada `Object::Stream`; `digest` menghitung
    /// lalu mengosongkan buffer seperti implementasi Java.
    fn message_digest_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getInstance", "(Ljava/lang/String;)Ljava/security/MessageDigest;") => Some((|| {
                let algorithm = Self::string_arg(heap, args, 0)?;
                let normalized = Self::digest_algorithm(&algorithm)?;
                Ok(heap.alloc(Object::Stream {
                    type_name: "Ljava/security/MessageDigest;".to_string(),
                    name: normalized,
                    data: Vec::new(),
                    position: 0,
                }))
            })(
            )),
            ("update", "([B)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let bytes = Self::byte_array_arg(heap, args, 1)?;
                Self::stream_append(heap, reference, &bytes)?;
                Ok(Value::Int(0))
            })()),
            ("update", "([BII)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let bytes = Self::byte_array_arg(heap, args, 1)?;
                let offset = usize::try_from(Self::int_arg(args, 2)?)
                    .map_err(|_| "offset negatif".to_string())?;
                let length = usize::try_from(Self::int_arg(args, 3)?)
                    .map_err(|_| "panjang negatif".to_string())?;
                let end = offset.saturating_add(length);
                let slice = bytes
                    .get(offset..end)
                    .ok_or_else(|| format!("rentang {offset}..{end} melebihi {}", bytes.len()))?;
                Self::stream_append(heap, reference, slice)?;
                Ok(Value::Int(0))
            })()),
            ("update", "(I)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let byte = Self::int_arg(args, 1)?;
                Self::stream_append(heap, reference, &[(byte & 0xFF) as u8])?;
                Ok(Value::Int(0))
            })()),
            ("digest", "()[B") => Some((|| {
                let reference = Self::this_ref(args)?;
                let (algorithm, data) = match heap.get(reference) {
                    Some(Object::Stream { name, data, .. }) => (name.clone(), data.clone()),
                    _ => return Err("objek bukan MessageDigest".to_string()),
                };
                let digest = Self::digest_bytes(&algorithm, &data)?;
                if let Some(Object::Stream { data, position, .. }) = heap.get_mut(reference) {
                    data.clear();
                    *position = 0;
                }
                Ok(Self::byte_array(heap, &digest))
            })()),
            ("digest", "([B)I") => Some((|| {
                let reference = Self::this_ref(args)?;
                let target = args
                    .get(1)
                    .and_then(|value| value.as_ref())
                    .ok_or("digest butuh array tujuan")?;
                let (algorithm, data) = match heap.get(reference) {
                    Some(Object::Stream { name, data, .. }) => (name.clone(), data.clone()),
                    _ => return Err("objek bukan MessageDigest".to_string()),
                };
                let digest = Self::digest_bytes(&algorithm, &data)?;
                let written = Self::fill_byte_array(heap, target, &digest)?;
                if let Some(Object::Stream { data, position, .. }) = heap.get_mut(reference) {
                    data.clear();
                    *position = 0;
                }
                Ok(Value::Int(written as i32))
            })()),
            ("reset", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                if let Some(Object::Stream { data, position, .. }) = heap.get_mut(reference) {
                    data.clear();
                    *position = 0;
                    Ok(Value::Int(0))
                } else {
                    Err("objek bukan MessageDigest".to_string())
                }
            })()),
            ("getAlgorithm", "()Ljava/lang/String;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let algorithm = match heap.get(reference) {
                    Some(Object::Stream { name, .. }) => name.clone(),
                    _ => return Err("objek bukan MessageDigest".to_string()),
                };
                Ok(heap.alloc(Object::String(algorithm)))
            })()),
            ("getDigestLength", "()I") => Some((|| {
                let reference = Self::this_ref(args)?;
                let algorithm = match heap.get(reference) {
                    Some(Object::Stream { name, .. }) => name.clone(),
                    _ => return Err("objek bukan MessageDigest".to_string()),
                };
                Ok(Value::Int(Self::digest_length(&algorithm) as i32))
            })()),
            ("isEqual", "([B[B)Z") => Some((|| {
                let left = Self::byte_array_arg(heap, args, 0)?;
                let right = Self::byte_array_arg(heap, args, 1)?;
                Ok(Value::Int(i32::from(left == right)))
            })()),
            _ => None,
        }
    }

    /// Nama algoritma yang dinormalkan ke bentuk tanpa tanda hubung.
    fn digest_algorithm(name: &str) -> Result<String, String> {
        let normalized: String = name
            .chars()
            .filter(|character| *character != '-' && *character != '_')
            .collect::<String>()
            .to_ascii_uppercase();
        match normalized.as_str() {
            "MD5" | "SHA1" | "SHA256" | "SHA512" => Ok(normalized),
            _ => Err(NativeException::new(
                "Ljava/security/NoSuchAlgorithmException;",
                format!("algoritma tidak didukung: {name}"),
            )
            .encode()),
        }
    }

    fn digest_length(algorithm: &str) -> usize {
        match algorithm {
            "MD5" => 16,
            "SHA1" => 20,
            "SHA256" => 32,
            _ => 64,
        }
    }

    fn digest_bytes(algorithm: &str, data: &[u8]) -> Result<Vec<u8>, String> {
        match algorithm {
            "MD5" => Ok(md5::compute(data).0.to_vec()),
            "SHA1" => {
                let mut hasher = sha1::Sha1::new();
                hasher.update(data);
                Ok(hasher.finalize().to_vec())
            }
            "SHA256" => {
                let mut hasher = sha2::Sha256::new();
                hasher.update(data);
                Ok(hasher.finalize().to_vec())
            }
            "SHA512" => {
                let mut hasher = sha2::Sha512::new();
                hasher.update(data);
                Ok(hasher.finalize().to_vec())
            }
            _ => Err(format!("algoritma tidak didukung: {algorithm}")),
        }
    }

    fn byte_array(heap: &mut Heap, bytes: &[u8]) -> Value {
        let elements = bytes
            .iter()
            .map(|byte| Value::Int(i32::from(*byte)))
            .collect();
        heap.alloc(Object::Array {
            element_type: "B".to_string(),
            elements,
        })
    }

    fn fill_byte_array(heap: &mut Heap, reference: u32, bytes: &[u8]) -> Result<usize, String> {
        match heap.get_mut(reference) {
            Some(Object::Array { elements, .. }) => {
                let written = elements.len().min(bytes.len());
                for (index, byte) in bytes.iter().take(written).enumerate() {
                    elements[index] = Value::Int(i32::from(*byte));
                }
                Ok(written)
            }
            _ => Err("argumen tujuan bukan array".to_string()),
        }
    }

    /// `WifiManager` virtual: tidak ada radio, jadi status dilaporkan mati dan
    /// identitas koneksi memakai placeholder seperti Android modern.
    fn wifi_manager_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getConnectionInfo", "()Landroid/net/wifi/WifiInfo;") => Some(Ok(
                Self::framework_instance(heap, "Landroid/net/wifi/WifiInfo;"),
            )),
            ("isWifiEnabled", "()Z") => Some(Ok(Value::Int(0))),
            ("setWifiEnabled", "(Z)Z") => Some(Ok(Value::Int(0))),
            ("getWifiState", "()I") => Some(Ok(Value::Int(1))),
            ("startScan", "()Z") => Some(Ok(Value::Int(0))),
            ("getScanResults", "()Ljava/util/List;") => Some(Ok(heap.alloc(Object::List {
                type_name: "Ljava/util/ArrayList;".to_owned(),
                elements: Vec::new(),
            }))),
            _ => None,
        }
    }

    /// `WifiInfo` virtual: SSID dan MAC tidak tersedia bagi aplikasi.
    fn wifi_info_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getMacAddress", "()Ljava/lang/String;") => {
                Some(Ok(heap.alloc(Object::String(VIRTUAL_MAC.to_owned()))))
            }
            ("getSSID", "()Ljava/lang/String;") => {
                Some(Ok(heap.alloc(Object::String("<unknown ssid>".to_owned()))))
            }
            ("getBSSID", "()Ljava/lang/String;") => {
                Some(Ok(heap.alloc(Object::String(VIRTUAL_MAC.to_owned()))))
            }
            ("getIpAddress", "()I") | ("getLinkSpeed", "()I") | ("getRssi", "()I") => {
                Some(Ok(Value::Int(0)))
            }
            ("getNetworkId", "()I") => Some(Ok(Value::Int(-1))),
            _ => None,
        }
    }

    /// `TelephonyManager` virtual: tidak ada SIM, semua pengenal dilaporkan
    /// kosong seperti perilaku Android untuk aplikasi tanpa izin.
    fn telephony_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getDeviceId", "()Ljava/lang/String;")
            | ("getImei", "()Ljava/lang/String;")
            | ("getMeid", "()Ljava/lang/String;")
            | ("getSimSerialNumber", "()Ljava/lang/String;")
            | ("getSubscriberId", "()Ljava/lang/String;")
            | ("getLine1Number", "()Ljava/lang/String;")
            | ("getNetworkOperator", "()Ljava/lang/String;")
            | ("getNetworkOperatorName", "()Ljava/lang/String;")
            | ("getSimOperator", "()Ljava/lang/String;")
            | ("getSimOperatorName", "()Ljava/lang/String;")
            | ("getSimCountryIso", "()Ljava/lang/String;")
            | ("getNetworkCountryIso", "()Ljava/lang/String;") => {
                Some(Ok(heap.alloc(Object::String(String::new()))))
            }
            ("getPhoneType", "()I") | ("getNetworkType", "()I") | ("getDataState", "()I") => {
                Some(Ok(Value::Int(0)))
            }
            ("getSimState", "()I") => Some(Ok(Value::Int(1))),
            _ => None,
        }
    }

    /// Instance framework tanpa field tersimpan; nilainya dipasok lewat
    /// `field_default` ketika kode aplikasi melakukan `iget`.
    fn framework_instance(heap: &mut Heap, type_name: &str) -> Value {
        heap.alloc(Object::Instance {
            type_name: type_name.to_string(),
            fields: std::collections::HashMap::new(),
        })
    }

    /// Field `versionCode` DEX bertipe `int`; nilai di luar rentang dijepit
    /// agar tidak membungkus menjadi angka negatif yang menyesatkan.
    fn clamp_int(value: Option<i64>) -> i32 {
        match value {
            Some(value) => {
                i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
            }
            None => 0,
        }
    }

    fn opt_string(heap: &mut Heap, text: Option<&str>) -> Value {
        match text {
            Some(text) => heap.alloc(Object::String(text.to_string())),
            None => Value::Int(0),
        }
    }

    /// `android.content.pm.PackageManager` sebatas metadata manifest.
    fn pm_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getPackageInfo", "(Ljava/lang/String;I)Landroid/content/pm/PackageInfo;") => {
                Some(Ok(Self::framework_instance(
                    heap,
                    "Landroid/content/pm/PackageInfo;",
                )))
            }
            ("getApplicationInfo", "(Ljava/lang/String;I)Landroid/content/pm/ApplicationInfo;") => {
                Some(Ok(Self::framework_instance(
                    heap,
                    "Landroid/content/pm/ApplicationInfo;",
                )))
            }
            ("getInstalledPackages", "(I)Ljava/util/List;") => {
                let info = Self::framework_instance(heap, "Landroid/content/pm/PackageInfo;");
                Some(Ok(heap.alloc(Object::List {
                    type_name: "Ljava/util/List;".to_string(),
                    elements: vec![info],
                })))
            }
            ("getPackageName", "()Ljava/lang/String;") => {
                Some(Ok(Self::opt_string(heap, self.manifest.package.as_deref())))
            }
            _ => None,
        }
    }

    fn bundle_lookup(heap: &Heap, reference: u32, key: Value) -> Option<Value> {
        match heap.get(reference)? {
            Object::Map { entries, .. } => entries
                .iter()
                .find(|(candidate, _)| Self::values_equal(heap, *candidate, key))
                .map(|(_, value)| *value),
            _ => None,
        }
    }

    fn bundle_put(heap: &mut Heap, reference: u32, key: Value, value: Value) -> Result<(), String> {
        let position = match heap.get(reference) {
            Some(Object::Map { entries, .. }) => entries
                .iter()
                .position(|(candidate, _)| Self::values_equal(heap, *candidate, key)),
            Some(_) => return Err("objek bukan Bundle".to_string()),
            None => return Err("objek Bundle tidak ada di heap".to_string()),
        };
        match heap.get_mut(reference) {
            Some(Object::Map { entries, .. }) => {
                match position {
                    Some(slot) => entries[slot].1 = value,
                    None => entries.push((key, value)),
                }
                Ok(())
            }
            _ => Err("objek bukan Bundle".to_string()),
        }
    }

    /// `android.os.Bundle` sebagai map kunci-nilai dalam memori.
    ///
    /// Bundle tidak pernah dikirim ke komponen lain di runtime ini; isinya
    /// hanya dipakai untuk meneruskan state di dalam satu proses.
    fn bundle_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("<init>", "()V") | ("<init>", "(I)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get_mut(reference) {
                    Some(slot) => {
                        *slot = Object::Map {
                            type_name: "Landroid/os/Bundle;".to_string(),
                            entries: Vec::new(),
                        };
                        Ok(Value::Int(0))
                    }
                    None => Err("objek Bundle tidak ada di heap".to_string()),
                }
            })()),
            (
                "putString"
                | "putInt"
                | "putLong"
                | "putBoolean"
                | "putFloat"
                | "putDouble"
                | "putByte"
                | "putShort"
                | "putChar"
                | "putSerializable"
                | "putParcelable"
                | "putStringArrayList"
                | "putIntegerArrayList"
                | "putParcelableArrayList",
                _,
            ) => Some((|| {
                let reference = Self::this_ref(args)?;
                let key = args.get(1).copied().ok_or("put butuh kunci")?;
                let value = args.get(2).copied().ok_or("put butuh nilai")?;
                Self::bundle_put(heap, reference, key, value)?;
                Ok(Value::Int(0))
            })()),
            ("getString", "(Ljava/lang/String;)Ljava/lang/String;")
            | ("getCharSequence", "(Ljava/lang/String;)Ljava/lang/CharSequence;")
            | ("getSerializable", "(Ljava/lang/String;)Ljava/io/Serializable;")
            | ("getParcelable", "(Ljava/lang/String;)Landroid/os/Parcelable;")
            | ("getStringArrayList", "(Ljava/lang/String;)Ljava/util/ArrayList;")
            | ("getIntegerArrayList", "(Ljava/lang/String;)Ljava/util/ArrayList;")
            | ("getParcelableArrayList", "(Ljava/lang/String;)Ljava/util/ArrayList;") => {
                Some((|| {
                    let reference = Self::this_ref(args)?;
                    let key = args.get(1).copied().ok_or("get butuh kunci")?;
                    Ok(Self::bundle_lookup(heap, reference, key).unwrap_or(Value::Int(0)))
                })())
            }
            ("getString", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;") => {
                Some((|| {
                    let reference = Self::this_ref(args)?;
                    let key = args.get(1).copied().ok_or("get butuh kunci")?;
                    let fallback = args.get(2).copied().unwrap_or(Value::Int(0));
                    Ok(Self::bundle_lookup(heap, reference, key).unwrap_or(fallback))
                })())
            }
            ("getInt", "(Ljava/lang/String;)I") => Some(Self::bundle_int(heap, args, None)),
            ("getInt", "(Ljava/lang/String;I)I") => Some(Self::bundle_int(heap, args, Some(2))),
            ("getLong", "(Ljava/lang/String;)J") => {
                Some(Self::bundle_wide(heap, args, None).map(Value::from_long))
            }
            ("getLong", "(Ljava/lang/String;J)J") => {
                Some(Self::bundle_wide(heap, args, Some(2)).map(Value::from_long))
            }
            ("getBoolean", "(Ljava/lang/String;)Z") => Some(Self::bundle_int(heap, args, None)),
            ("getBoolean", "(Ljava/lang/String;Z)Z") => Some(Self::bundle_int(heap, args, Some(2))),
            ("containsKey", "(Ljava/lang/String;)Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                let key = args.get(1).copied().ok_or("containsKey butuh kunci")?;
                Ok(Value::Int(i32::from(
                    Self::bundle_lookup(heap, reference, key).is_some(),
                )))
            })()),
            ("isEmpty", "()Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get(reference) {
                    Some(Object::Map { entries, .. }) => {
                        Ok(Value::Int(i32::from(entries.is_empty())))
                    }
                    _ => Err("objek bukan Bundle".to_string()),
                }
            })()),
            ("size", "()I") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get(reference) {
                    Some(Object::Map { entries, .. }) => Ok(Value::Int(entries.len() as i32)),
                    _ => Err("objek bukan Bundle".to_string()),
                }
            })()),
            ("remove", "(Ljava/lang/String;)V") | ("clear", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let key = args.get(1).copied();
                let position = match (key, heap.get(reference)) {
                    (Some(key), Some(Object::Map { entries, .. })) => entries
                        .iter()
                        .position(|(candidate, _)| Self::values_equal(heap, *candidate, key)),
                    (None, Some(Object::Map { .. })) => None,
                    (_, Some(_)) => return Err("objek bukan Bundle".to_string()),
                    (_, None) => return Err("objek Bundle tidak ada di heap".to_string()),
                };
                match heap.get_mut(reference) {
                    Some(Object::Map { entries, .. }) => {
                        match (key, position) {
                            (Some(_), Some(slot)) => {
                                entries.remove(slot);
                            }
                            (Some(_), None) => {}
                            (None, _) => entries.clear(),
                        }
                        Ok(Value::Int(0))
                    }
                    _ => Err("objek bukan Bundle".to_string()),
                }
            })()),
            _ => None,
        }
    }

    /// `fallback_at` menunjuk posisi argumen nilai bawaan pada overload
    /// dua-argumen; `None` berarti overload satu-argumen (bawaan nol).
    /// `<meta-data>` selalu berupa teks di manifest, sedangkan kode aplikasi
    /// meminta tipe konkret; teks numerik diterima sebagai angka.
    fn as_int_or_text(heap: &Heap, value: Value) -> Option<i32> {
        match value {
            Value::Int(number) => Some(number),
            Value::Ref(reference) => heap
                .string(reference)
                .and_then(|text| text.trim().parse::<i32>().ok()),
            Value::Wide(_) => None,
        }
    }

    fn bundle_int(
        heap: &Heap,
        args: &[Value],
        fallback_at: Option<usize>,
    ) -> Result<Value, String> {
        let reference = Self::this_ref(args)?;
        let key = args.get(1).copied().ok_or("get butuh kunci")?;
        let fallback = fallback_at
            .and_then(|index| args.get(index))
            .and_then(|value| value.as_int())
            .unwrap_or(0);
        let value = Self::bundle_lookup(heap, reference, key)
            .and_then(|value| Self::as_int_or_text(heap, value))
            .unwrap_or(fallback);
        Ok(Value::Int(value))
    }

    fn bundle_wide(heap: &Heap, args: &[Value], fallback_at: Option<usize>) -> Result<i64, String> {
        let reference = Self::this_ref(args)?;
        let key = args.get(1).copied().ok_or("get butuh kunci")?;
        let fallback = fallback_at
            .and_then(|index| args.get(index))
            .and_then(|value| value.as_long())
            .unwrap_or(0);
        Ok(Self::bundle_lookup(heap, reference, key)
            .and_then(|value| match value {
                Value::Wide(bits) => Some(bits as i64),
                other => Self::as_int_or_text(heap, other).map(i64::from),
            })
            .unwrap_or(fallback))
    }

    /// Aliran berkas yang disediakan runtime.
    ///
    /// Isi berkas dibaca utuh saat dibuka (dibatasi host), dan hanya ditulis
    /// kembali ke disk saat `flush`/`close`. Aliran yang tidak pernah ditutup
    /// kehilangan perubahannya, sama seperti berkas yang belum di-flush.
    /// `java.lang.reflect.Method` yang dibuat interpreter.
    fn reflect_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getName", "()Ljava/lang/String;") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get(reference) {
                    Some(Object::Method { name, .. }) => {
                        Ok(heap.alloc(Object::String(name.clone())))
                    }
                    _ => Err("objek bukan Method".to_string()),
                }
            })()),
            ("getDeclaringClass", "()Ljava/lang/Class;") => Some((|| {
                let reference = Self::this_ref(args)?;
                match heap.get(reference) {
                    Some(Object::Method { class, .. }) => Ok(heap.alloc(Object::Class {
                        descriptor: class.clone(),
                    })),
                    _ => Err("objek bukan Method".to_string()),
                }
            })()),
            ("getParameterTypes", "()[Ljava/lang/Class;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let parameters = match heap.get(reference) {
                    Some(Object::Method { parameters, .. }) => parameters.clone(),
                    _ => return Err("objek bukan Method".to_string()),
                };
                let mut elements = Vec::with_capacity(parameters.len());
                for descriptor in parameters {
                    elements.push(heap.alloc(Object::Class { descriptor }));
                }
                Ok(heap.alloc(Object::Array {
                    element_type: "Ljava/lang/Class;".to_string(),
                    elements,
                }))
            })()),
            // Tidak ada pemeriksaan akses di runtime ini.
            ("setAccessible", "(Z)V") => Some(Ok(Value::Int(0))),
            ("isAccessible", "()Z") => Some(Ok(Value::Int(1))),
            _ => None,
        }
    }

    /// `android.text.TextUtils` untuk pemeriksaan teks yang umum dipakai.
    fn text_utils_method(
        heap: &Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            // Android menganggap null sebagai teks kosong.
            ("isEmpty", "(Ljava/lang/CharSequence;)Z") => Some(Ok(Value::Int(i32::from(
                args.first()
                    .copied()
                    .and_then(|value| Self::char_sequence_text(heap, value))
                    .unwrap_or_default()
                    .is_empty(),
            )))),
            ("equals", "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Z") => {
                let left = args
                    .first()
                    .copied()
                    .and_then(|value| Self::char_sequence_text(heap, value));
                let right = args
                    .get(1)
                    .copied()
                    .and_then(|value| Self::char_sequence_text(heap, value));
                Some(Ok(Value::Int(i32::from(left == right))))
            }
            _ => None,
        }
    }

    /// `java.lang.Thread` minimal: satu thread utama tanpa penjadwalan nyata.
    fn thread_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("currentThread", "()Ljava/lang/Thread;") => {
                let name = heap.alloc(Object::String("main".to_string()));
                let mut fields = std::collections::HashMap::new();
                fields.insert(0, name);
                Some(Ok(heap.alloc(Object::Instance {
                    type_name: "Ljava/lang/Thread;".to_string(),
                    fields,
                })))
            }
            ("getName", "()Ljava/lang/String;") => Some((|| {
                let reference = Self::this_ref(args)?;
                Ok(Self::thread_name(heap, reference).unwrap_or(Value::Int(0)))
            })()),
            ("setName", "(Ljava/lang/String;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let name = args.get(1).copied().ok_or("setName butuh nama")?;
                match heap.get_mut(reference) {
                    Some(Object::Instance { fields, .. }) => {
                        fields.insert(0, name);
                        Ok(Value::Int(0))
                    }
                    _ => Err("objek bukan Thread".to_string()),
                }
            })()),
            ("getId", "()J") => Some(Ok(Value::from_long(1))),
            ("toString", "()Ljava/lang/String;") => {
                let name = Self::this_ref(args)
                    .ok()
                    .and_then(|reference| Self::thread_name(heap, reference))
                    .and_then(|value| value.as_ref())
                    .and_then(|text| heap.string(text))
                    .unwrap_or("main")
                    .to_owned();
                Some(Ok(
                    heap.alloc(Object::String(format!("Thread[{name},5,main]")))
                ))
            }
            // Runtime ini kooperatif dan tidak menjadwalkan thread; sleep hanya
            // menerima panggilan tanpa memblokir proses host.
            ("sleep", "(J)V") => Some(Ok(Value::Int(0))),
            ("interrupted", "()Z") | ("isInterrupted", "()Z") => Some(Ok(Value::Int(0))),
            // Runtime ini tidak punya penangan eksepsi global nyata; handler
            // yang dipasang dicatat sebagai no-op dan getter mengembalikan
            // objek Thread$UncaughtExceptionHandler kosong (bukan null) agar
            // kode aplikasi tidak mendapat NullReference saat memakainya.
            ("setDefaultUncaughtExceptionHandler", _) | ("setUncaughtExceptionHandler", _) => {
                Some(Ok(Value::Int(0)))
            }
            ("getDefaultUncaughtExceptionHandler", _) | ("getUncaughtExceptionHandler", _) => {
                let handler = heap.alloc(Object::Instance {
                    type_name: "Ljava/lang/Thread$UncaughtExceptionHandler;".to_string(),
                    fields: std::collections::HashMap::new(),
                });
                Some(Ok(handler))
            }
            ("getStackTrace", "()[Ljava/lang/StackTraceElement;") => {
                Some(Ok(heap.alloc(Object::Array {
                    element_type: "Ljava/lang/StackTraceElement;".to_string(),
                    elements: Vec::new(),
                })))
            }
            ("uncaughtException", "(Ljava/lang/Thread;Ljava/lang/Throwable;)V") => {
                Some(Ok(Value::Int(0)))
            }
            _ => None,
        }
    }

    fn thread_name(heap: &Heap, reference: u32) -> Option<Value> {
        match heap.get(reference)? {
            Object::Instance { fields, .. } => fields.get(&0).copied(),
            _ => None,
        }
    }

    /// `java.lang.Long`, termasuk boxing.
    fn long_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("parseLong", "(Ljava/lang/String;)J") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                text.trim()
                    .parse::<i64>()
                    .map(Value::from_long)
                    .map_err(|error| format!("bukan angka long: {error}"))
            })()),
            ("parseLong", "(Ljava/lang/String;I)J") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let radix = Self::int_arg(args, 1)?;
                let radix = u32::try_from(radix)
                    .ok()
                    .filter(|radix| (2..=36).contains(radix))
                    .ok_or_else(|| format!("radix tidak valid: {radix}"))?;
                i64::from_str_radix(text.trim(), radix)
                    .map(Value::from_long)
                    .map_err(|error| format!("bukan angka long: {error}"))
            })()),
            ("toString", "(J)Ljava/lang/String;") => Some((|| {
                let value = Self::long_arg(args, 0)?;
                Ok(heap.alloc(Object::String(value.to_string())))
            })()),
            ("valueOf", "(J)Ljava/lang/Long;") => Some((|| {
                Ok(heap.alloc(Object::Boxed {
                    type_name: "Ljava/lang/Long;".to_string(),
                    value: Value::from_long(Self::long_arg(args, 0)?),
                }))
            })()),
            ("valueOf", "(Ljava/lang/String;)Ljava/lang/Long;") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let parsed = text
                    .trim()
                    .parse::<i64>()
                    .map_err(|error| format!("bukan angka long: {error}"))?;
                Ok(heap.alloc(Object::Boxed {
                    type_name: "Ljava/lang/Long;".to_string(),
                    value: Value::from_long(parsed),
                }))
            })()),
            ("longValue", "()J") => Some((|| {
                let value = Self::boxed_value(heap, args)?;
                Ok(Value::from_long(match value {
                    Value::Wide(bits) => bits as i64,
                    Value::Int(number) => i64::from(number),
                    Value::Ref(_) => return Err("nilai boxed bukan angka".to_string()),
                }))
            })()),
            ("intValue", "()I") => Some((|| {
                let value = Self::boxed_value(heap, args)?;
                match value {
                    Value::Wide(bits) => Ok(Value::Int(bits as i64 as i32)),
                    Value::Int(number) => Ok(Value::Int(number)),
                    Value::Ref(_) => Err("nilai boxed bukan angka".to_string()),
                }
            })()),
            ("toString", "()Ljava/lang/String;") => Some((|| {
                let value = Self::boxed_value(heap, args)?;
                let text = match value {
                    Value::Wide(bits) => (bits as i64).to_string(),
                    Value::Int(number) => number.to_string(),
                    Value::Ref(_) => return Err("nilai boxed bukan angka".to_string()),
                };
                Ok(heap.alloc(Object::String(text)))
            })()),
            _ => None,
        }
    }

    /// `android.os.Process` sebatas identitas proses.
    ///
    /// Aplikasi memang berjalan di proses host ini, jadi pid/tid yang
    /// dilaporkan adalah pid/tid proses Arunika sendiri.
    fn process_method(method: &str, signature: &str) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("myPid", "()I") | ("myTid", "()I") => Some(Ok(Value::Int(std::process::id() as i32))),
            _ => None,
        }
    }

    /// `java.net.URLEncoder`/`URLDecoder` dengan aturan application/x-www-form-urlencoded.
    fn url_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("encode", "(Ljava/lang/String;)Ljava/lang/String;") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                Ok(heap.alloc(Object::String(Self::url_encode(&text))))
            })()),
            ("decode", "(Ljava/lang/String;)Ljava/lang/String;") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                Ok(heap.alloc(Object::String(Self::url_decode(&text)?)))
            })()),
            _ => None,
        }
    }

    fn url_encode(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for byte in text.as_bytes() {
            match byte {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'*' | b'_' => {
                    out.push(*byte as char);
                }
                b' ' => out.push('+'),
                other => {
                    use std::fmt::Write as _;
                    let _ = write!(out, "%{other:02X}");
                }
            }
        }
        out
    }

    fn url_decode(text: &str) -> Result<String, String> {
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'+' => {
                    out.push(b' ');
                    index += 1;
                }
                b'%' => {
                    let hex = bytes
                        .get(index + 1..index + 3)
                        .ok_or("escape persen tidak lengkap")?;
                    let hex = std::str::from_utf8(hex)
                        .map_err(|_| "escape persen bukan hex".to_string())?;
                    let byte =
                        u8::from_str_radix(hex, 16).map_err(|_| format!("%{hex} bukan hex"))?;
                    out.push(byte);
                    index += 3;
                }
                byte => {
                    out.push(byte);
                    index += 1;
                }
            }
        }
        String::from_utf8(out).map_err(|_| "hasil decode bukan UTF-8".to_string())
    }

    /// `java.util.UUID` dengan representasi teks pada field 0.
    fn uuid_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("randomUUID", "()Ljava/util/UUID;") => {
                Some(Ok(Self::make_uuid(heap, Self::random_uuid_text())))
            }
            ("fromString", "(Ljava/lang/String;)Ljava/util/UUID;") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                if text.len() != 36 {
                    return Err(format!("UUID tidak valid: {text}"));
                }
                Ok(Self::make_uuid(heap, text.to_ascii_lowercase()))
            })()),
            ("toString", "()Ljava/lang/String;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let text = Self::uuid_text(heap, reference)
                    .ok_or_else(|| "objek bukan UUID".to_string())?;
                Ok(heap.alloc(Object::String(text)))
            })()),
            ("equals", "(Ljava/lang/Object;)Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                let left = Self::uuid_text(heap, reference)
                    .ok_or_else(|| "objek bukan UUID".to_string())?;
                let right = args
                    .get(1)
                    .and_then(|value| value.as_ref())
                    .and_then(|other| Self::uuid_text(heap, other));
                Ok(Value::Int(i32::from(
                    right.as_deref() == Some(left.as_str()),
                )))
            })()),
            ("hashCode", "()I") => Some((|| {
                let reference = Self::this_ref(args)?;
                let text = Self::uuid_text(heap, reference)
                    .ok_or_else(|| "objek bukan UUID".to_string())?;
                let mut hash = 0i32;
                for unit in text.encode_utf16() {
                    hash = hash.wrapping_mul(31).wrapping_add(i32::from(unit));
                }
                Ok(Value::Int(hash))
            })()),
            _ => None,
        }
    }

    fn make_uuid(heap: &mut Heap, text: String) -> Value {
        let stored = heap.alloc(Object::String(text));
        let mut fields = std::collections::HashMap::new();
        fields.insert(0, stored);
        heap.alloc(Object::Instance {
            type_name: "Ljava/util/UUID;".to_string(),
            fields,
        })
    }

    fn uuid_text(heap: &Heap, reference: u32) -> Option<String> {
        match heap.get(reference)? {
            Object::Instance { fields, .. } => fields
                .get(&0)
                .and_then(|value| value.as_ref())
                .and_then(|text| heap.string(text))
                .map(|text| text.to_owned()),
            _ => None,
        }
    }

    /// UUID versi 4 dari entropi yang disediakan std (bukan PRNG buatan sendiri).
    fn random_uuid_text() -> String {
        use std::hash::{BuildHasher, Hasher};

        let mut bytes = [0u8; 16];
        for (index, chunk) in bytes.chunks_mut(8).enumerate() {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_usize(index);
            chunk.copy_from_slice(&hasher.finish().to_le_bytes());
        }
        bytes[6] = (bytes[6] & 0x0F) | 0x40;
        bytes[8] = (bytes[8] & 0x3F) | 0x80;

        let mut hex = String::with_capacity(32);
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )
    }

    /// Eksepsi Java yang dibuat runtime hanya membawa pesan pada field 0.
    fn throwable_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("getMessage", "()Ljava/lang/String;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let message = match heap.get(reference) {
                    Some(Object::Instance { fields, .. }) => {
                        fields.get(&0).copied().unwrap_or(Value::Int(0))
                    }
                    _ => return Err("objek bukan eksepsi".to_string()),
                };
                Ok(message)
            })()),
            ("toString", "()Ljava/lang/String;") => Some((|| {
                let reference = Self::this_ref(args)?;
                let (type_name, message) = match heap.get(reference) {
                    Some(Object::Instance { type_name, fields }) => {
                        let message = fields
                            .get(&0)
                            .and_then(|value| value.as_ref())
                            .and_then(|text| heap.string(text))
                            .map(|text| text.to_owned());
                        (type_name.clone(), message)
                    }
                    _ => return Err("objek bukan eksepsi".to_string()),
                };
                let name = Self::java_name(&type_name);
                let text = match message {
                    Some(message) if !message.is_empty() => format!("{name}: {message}"),
                    _ => name,
                };
                Ok(heap.alloc(Object::String(text)))
            })()),
            ("printStackTrace", "()V") => Some(Ok(Value::Int(0))),
            _ => None,
        }
    }

    /// Ringkasan teks sebuah Throwable: nama Java diikuti pesan bila ada.
    ///
    /// Runtime ini belum menyimpan stack frame, jadi yang bisa dilaporkan
    /// hanya kelas dan pesan. String kosong berarti objek bukan Throwable.
    fn describe_throwable(heap: &Heap, object: &Object) -> String {
        match object {
            Object::Instance { type_name, fields } => {
                let message = fields
                    .get(&0)
                    .and_then(|value| value.as_ref())
                    .and_then(|text| heap.string(text));
                match message {
                    Some(message) if !message.is_empty() => {
                        format!("{}: {message}", Self::java_name(type_name))
                    }
                    _ => Self::java_name(type_name),
                }
            }
            _ => String::new(),
        }
    }

    fn stream_method(
        &self,
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("read", "()I") => Some((|| {
                let reference = Self::this_ref(args)?;
                let (data, position) = Self::stream_state(heap, reference)?;
                if position >= data.len() {
                    return Ok(Value::Int(-1));
                }
                let byte = data[position];
                Self::stream_advance(heap, reference, 1)?;
                Ok(Value::Int(i32::from(byte)))
            })()),
            ("read", "([B)I") => Some(Self::stream_read_into(heap, args, false)),
            ("read", "([BII)I") => Some(Self::stream_read_into(heap, args, true)),
            ("available", "()I") => Some((|| {
                let reference = Self::this_ref(args)?;
                let (data, position) = Self::stream_state(heap, reference)?;
                Ok(Value::Int((data.len().saturating_sub(position)) as i32))
            })()),
            ("write", "(I)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let byte = Self::int_arg(args, 1)?;
                Self::stream_append(heap, reference, &[(byte & 0xFF) as u8])?;
                Ok(Value::Int(0))
            })()),
            ("write", "([B)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let bytes = Self::byte_array_arg(heap, args, 1)?;
                Self::stream_append(heap, reference, &bytes)?;
                Ok(Value::Int(0))
            })()),
            ("write", "([BII)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let bytes = Self::byte_array_arg(heap, args, 1)?;
                let offset = usize::try_from(Self::int_arg(args, 2)?)
                    .map_err(|_| "offset negatif".to_string())?;
                let length = usize::try_from(Self::int_arg(args, 3)?)
                    .map_err(|_| "panjang negatif".to_string())?;
                let end = offset.saturating_add(length);
                let slice = bytes
                    .get(offset..end)
                    .ok_or_else(|| format!("rentang {offset}..{end} melebihi {}", bytes.len()))?;
                Self::stream_append(heap, reference, slice)?;
                Ok(Value::Int(0))
            })()),
            ("flush", "()V") | ("close", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                self.stream_persist(heap, reference)?;
                Ok(Value::Int(0))
            })()),
            ("getFD", "()Ljava/io/FileDescriptor;") => {
                Some(Err("FileDescriptor belum tersedia di runtime".to_string()))
            }
            _ => None,
        }
    }

    /// Pembaca/penulis teks berbasis berkas.
    ///
    /// Path Java dipetakan ke nama berkas di direktori data aplikasi dengan
    /// hanya memakai komponen terakhir, sehingga sandbox tetap berlaku. Isi
    /// berkas ditafsirkan sebagai UTF-8.
    fn text_stream_method(
        &self,
        heap: &mut Heap,
        class_descriptor: &str,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        let reader = class_descriptor.contains("Reader");
        match (method, signature) {
            ("<init>", "(Ljava/lang/String;)V") | ("<init>", "(Ljava/io/File;)V") => {
                Some(self.open_text_stream(heap, class_descriptor, args, reader, false))
            }
            ("<init>", "(Ljava/lang/String;Z)V") | ("<init>", "(Ljava/io/File;Z)V") => {
                Some(self.open_text_stream(heap, class_descriptor, args, reader, true))
            }
            ("<init>", "(Ljava/io/Reader;)V") | ("<init>", "(Ljava/io/Writer;)V") => {
                Some(Self::wrap_text_stream(heap, class_descriptor, args))
            }
            ("read", "()I") => Some(Self::text_read_char(heap, args)),
            ("read", "([C)I") => Some(Self::text_read_chars(heap, args)),
            ("readLine", "()Ljava/lang/String;") => Some(Self::text_read_line(heap, args)),
            ("ready", "()Z") => Some((|| {
                let reference = Self::this_ref(args)?;
                let (data, position) = Self::stream_state(heap, reference)?;
                Ok(Value::Int(i32::from(position < data.len())))
            })()),
            ("write", "(I)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let value = Self::int_arg(args, 1)?;
                let character = u32::try_from(value)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| format!("bukan karakter: {value}"))?;
                let mut buffer = [0u8; 4];
                Self::stream_append(
                    heap,
                    reference,
                    character.encode_utf8(&mut buffer).as_bytes(),
                )?;
                Ok(Value::Int(0))
            })()),
            ("write", "(Ljava/lang/String;)V") => Some((|| {
                let reference = Self::this_ref(args)?;
                let text = Self::string_arg(heap, args, 1)?;
                Self::stream_append(heap, reference, text.as_bytes())?;
                Ok(Value::Int(0))
            })()),
            ("write", "([C)V") | ("write", "([CII)V") => {
                Some(Self::text_write_chars(heap, args, signature == "([CII)V"))
            }
            ("newLine", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                Self::stream_append(heap, reference, b"\n")?;
                Ok(Value::Int(0))
            })()),
            ("flush", "()V") | ("close", "()V") => Some((|| {
                let reference = Self::this_ref(args)?;
                self.stream_persist(heap, reference)?;
                Ok(Value::Int(0))
            })()),
            _ => None,
        }
    }

    fn open_text_stream(
        &self,
        heap: &mut Heap,
        class_descriptor: &str,
        args: &[Value],
        reader: bool,
        append: bool,
    ) -> Result<Value, String> {
        let reference = Self::this_ref(args)?;
        let path = Self::path_arg(heap, args, 1)?;
        let name = path.rsplit('/').next().unwrap_or_default().to_string();
        let file_system = self.require_file_system()?;
        let data = if reader {
            file_system.read(&name).map_err(|error| {
                let class = match error {
                    arunika_sandbox_manager::SandboxError::PathEscape(_) => {
                        "Ljava/lang/SecurityException;"
                    }
                    _ => "Ljava/io/FileNotFoundException;",
                };
                NativeException::new(class, format!("{path}: {error}")).encode()
            })?
        } else if append {
            file_system.read(&name).unwrap_or_default()
        } else {
            Vec::new()
        };
        let position = if reader { 0 } else { data.len() };
        match heap.get_mut(reference) {
            Some(slot) => {
                *slot = Object::Stream {
                    type_name: class_descriptor.to_string(),
                    name,
                    data,
                    position,
                };
                Ok(Value::Int(0))
            }
            None => Err("objek aliran teks tidak ada di heap".to_string()),
        }
    }

    fn wrap_text_stream(
        heap: &mut Heap,
        class_descriptor: &str,
        args: &[Value],
    ) -> Result<Value, String> {
        let reference = Self::this_ref(args)?;
        let source = args
            .get(1)
            .and_then(|value| value.as_ref())
            .ok_or("pembungkus butuh aliran sumber")?;
        let (name, data, position) = match heap.get(source) {
            Some(Object::Stream {
                name,
                data,
                position,
                ..
            }) => (name.clone(), data.clone(), *position),
            _ => return Err("sumber bukan aliran berkas".to_string()),
        };
        match heap.get_mut(reference) {
            Some(slot) => {
                *slot = Object::Stream {
                    type_name: class_descriptor.to_string(),
                    name,
                    data,
                    position,
                };
                Ok(Value::Int(0))
            }
            None => Err("objek pembungkus tidak ada di heap".to_string()),
        }
    }

    fn path_arg(heap: &Heap, args: &[Value], index: usize) -> Result<String, String> {
        let reference = args
            .get(index)
            .and_then(|value| value.as_ref())
            .ok_or("argumen path harus objek")?;
        match heap.get(reference) {
            Some(Object::String(text)) => Ok(text.clone()),
            Some(Object::Instance { fields, .. }) => fields
                .get(&0)
                .and_then(|value| value.as_ref())
                .and_then(|text| heap.string(text))
                .map(|text| text.to_owned())
                .ok_or_else(|| "objek File tidak menyimpan path".to_string()),
            _ => Err("argumen path bukan String atau File".to_string()),
        }
    }

    fn text_read_char(heap: &mut Heap, args: &[Value]) -> Result<Value, String> {
        let reference = Self::this_ref(args)?;
        let (character, width) = {
            let (data, position) = Self::stream_state(heap, reference)?;
            let rest = &data[position.min(data.len())..];
            match String::from_utf8_lossy(rest).chars().next() {
                Some(character) => (character, character.len_utf8()),
                None => return Ok(Value::Int(-1)),
            }
        };
        Self::stream_advance(heap, reference, width)?;
        Ok(Value::Int(character as i32))
    }

    fn text_read_chars(heap: &mut Heap, args: &[Value]) -> Result<Value, String> {
        let reference = Self::this_ref(args)?;
        let array = args
            .get(1)
            .and_then(|value| value.as_ref())
            .ok_or("read butuh array tujuan")?;
        let capacity = heap
            .array_len(array)
            .ok_or_else(|| "argumen tujuan bukan array".to_string())?;
        let (chars, consumed) = {
            let (data, position) = Self::stream_state(heap, reference)?;
            let rest = &data[position.min(data.len())..];
            let chars: Vec<char> = String::from_utf8_lossy(rest)
                .chars()
                .take(capacity)
                .collect();
            let consumed: usize = chars.iter().map(|character| character.len_utf8()).sum();
            (chars, consumed)
        };
        if chars.is_empty() {
            return Ok(Value::Int(-1));
        }
        match heap.get_mut(array) {
            Some(Object::Array { elements, .. }) => {
                for (index, character) in chars.iter().enumerate() {
                    if index < elements.len() {
                        elements[index] = Value::Int(*character as i32);
                    }
                }
            }
            _ => return Err("argumen tujuan bukan array".to_string()),
        }
        Self::stream_advance(heap, reference, consumed)?;
        Ok(Value::Int(chars.len() as i32))
    }

    fn text_read_line(heap: &mut Heap, args: &[Value]) -> Result<Value, String> {
        let reference = Self::this_ref(args)?;
        let (line, consumed) = {
            let (data, position) = Self::stream_state(heap, reference)?;
            let rest = &data[position.min(data.len())..];
            match rest.iter().position(|byte| *byte == b'\n') {
                Some(end) => (
                    String::from_utf8_lossy(&rest[..end])
                        .trim_end_matches('\r')
                        .to_string(),
                    end + 1,
                ),
                None if rest.is_empty() => return Ok(Value::Int(0)),
                None => (
                    String::from_utf8_lossy(rest)
                        .trim_end_matches('\r')
                        .to_string(),
                    rest.len(),
                ),
            }
        };
        Self::stream_advance(heap, reference, consumed)?;
        Ok(heap.alloc(Object::String(line)))
    }

    fn text_write_chars(heap: &mut Heap, args: &[Value], ranged: bool) -> Result<Value, String> {
        let reference = Self::this_ref(args)?;
        let array = args
            .get(1)
            .and_then(|value| value.as_ref())
            .ok_or("write butuh array karakter")?;
        let (offset, length) = if ranged {
            let offset = usize::try_from(Self::int_arg(args, 2)?)
                .map_err(|_| "offset negatif".to_string())?;
            let length = usize::try_from(Self::int_arg(args, 3)?)
                .map_err(|_| "panjang negatif".to_string())?;
            (offset, length)
        } else {
            (
                0,
                heap.array_len(array)
                    .ok_or_else(|| "argumen bukan array".to_string())?,
            )
        };
        let text = {
            let Some(Object::Array { elements, .. }) = heap.get(array) else {
                return Err("argumen bukan array karakter".to_string());
            };
            let end = offset.saturating_add(length).min(elements.len());
            let slice = elements
                .get(offset..end)
                .ok_or_else(|| format!("rentang {offset}..{end} melebihi {}", elements.len()))?;
            slice
                .iter()
                .map(|value| {
                    value
                        .as_int()
                        .and_then(|code| u32::try_from(code).ok())
                        .and_then(char::from_u32)
                        .unwrap_or('\u{FFFD}')
                })
                .collect::<String>()
        };
        Self::stream_append(heap, reference, text.as_bytes())?;
        Ok(Value::Int(0))
    }

    fn stream_state(heap: &Heap, reference: u32) -> Result<(&[u8], usize), String> {
        match heap.get(reference) {
            Some(Object::Stream { data, position, .. }) => Ok((data, *position)),
            _ => Err("objek bukan aliran berkas".to_string()),
        }
    }

    fn stream_advance(heap: &mut Heap, reference: u32, count: usize) -> Result<(), String> {
        match heap.get_mut(reference) {
            Some(Object::Stream { position, .. }) => {
                *position += count;
                Ok(())
            }
            _ => Err("objek bukan aliran berkas".to_string()),
        }
    }

    fn stream_append(heap: &mut Heap, reference: u32, bytes: &[u8]) -> Result<(), String> {
        match heap.get_mut(reference) {
            Some(Object::Stream { data, position, .. }) => {
                data.extend_from_slice(bytes);
                *position = data.len();
                Ok(())
            }
            _ => Err("objek bukan aliran berkas".to_string()),
        }
    }

    /// Menyalin isi aliran ke array byte milik pemanggil.
    fn stream_read_into(heap: &mut Heap, args: &[Value], ranged: bool) -> Result<Value, String> {
        let reference = Self::this_ref(args)?;
        let array = args
            .get(1)
            .and_then(|value| value.as_ref())
            .ok_or("read butuh array tujuan")?;
        let capacity = heap
            .array_len(array)
            .ok_or_else(|| "argumen tujuan bukan array".to_string())?;
        let (offset, length) = if ranged {
            let offset = usize::try_from(Self::int_arg(args, 2)?)
                .map_err(|_| "offset negatif".to_string())?;
            let length = usize::try_from(Self::int_arg(args, 3)?)
                .map_err(|_| "panjang negatif".to_string())?;
            (offset, length)
        } else {
            (0, capacity)
        };
        let end = offset.saturating_add(length);
        if end > capacity {
            return Err(format!(
                "rentang {offset}..{end} melebihi panjang array {capacity}"
            ));
        }

        let (available, position) = {
            let (data, position) = Self::stream_state(heap, reference)?;
            (data.len().saturating_sub(position), position)
        };
        if available == 0 {
            return Ok(Value::Int(-1));
        }
        let count = length.min(available);
        let chunk: Vec<u8> = {
            let (data, _) = Self::stream_state(heap, reference)?;
            data[position..position + count].to_vec()
        };
        if let Some(Object::Array { elements, .. }) = heap.get_mut(array) {
            for (slot, byte) in elements[offset..offset + count]
                .iter_mut()
                .zip(chunk.iter())
            {
                *slot = Value::Int(i32::from(*byte));
            }
        }
        Self::stream_advance(heap, reference, count)?;
        Ok(Value::Int(count as i32))
    }

    /// Isi array byte sebagai vektor; nilai di luar rentang byte dijepit.
    fn byte_array_arg(heap: &Heap, args: &[Value], index: usize) -> Result<Vec<u8>, String> {
        let reference = args
            .get(index)
            .and_then(|value| value.as_ref())
            .ok_or("argumen harus array byte")?;
        match heap.get(reference) {
            Some(Object::Array { elements, .. }) => Ok(elements
                .iter()
                .map(|value| value.as_int().unwrap_or(0).clamp(0, 255) as u8)
                .collect()),
            _ => Err("argumen bukan array byte".to_string()),
        }
    }

    fn stream_persist(&self, heap: &Heap, reference: u32) -> Result<(), String> {
        let (name, data) = match heap.get(reference) {
            Some(Object::Stream {
                type_name,
                name,
                data,
                ..
            }) => {
                let writable = type_name.ends_with("FileOutputStream;")
                    || type_name.ends_with("FileWriter;")
                    || type_name.ends_with("BufferedWriter;");
                if !writable {
                    return Ok(());
                }
                (name.clone(), data.clone())
            }
            _ => return Err("objek bukan aliran berkas".to_string()),
        };
        self.require_file_system()?
            .write(&name, &data)
            .map_err(|error| error.to_string())
    }

    fn system_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("arraycopy", "(Ljava/lang/Object;ILjava/lang/Object;II)V") => Some((|| {
                let source = args
                    .first()
                    .and_then(|value| value.as_ref())
                    .ok_or("arraycopy: sumber bukan array")?;
                let source_pos = Self::int_arg(args, 1)?;
                let destination = args
                    .get(2)
                    .and_then(|value| value.as_ref())
                    .ok_or("arraycopy: tujuan bukan array")?;
                let destination_pos = Self::int_arg(args, 3)?;
                let length = Self::int_arg(args, 4)?;

                let source_pos = usize::try_from(source_pos)
                    .map_err(|_| "arraycopy: posisi sumber negatif".to_string())?;
                let destination_pos = usize::try_from(destination_pos)
                    .map_err(|_| "arraycopy: posisi tujuan negatif".to_string())?;
                let length = usize::try_from(length)
                    .map_err(|_| "arraycopy: panjang negatif".to_string())?;

                let copied = match heap.get(source) {
                    Some(Object::Array { elements, .. }) => {
                        let end = source_pos.saturating_add(length);
                        if end > elements.len() {
                            return Err(format!(
                                "arraycopy: rentang sumber {source_pos}..{end} melebihi {} elemen",
                                elements.len()
                            ));
                        }
                        elements[source_pos..end].to_vec()
                    }
                    _ => return Err("arraycopy: sumber bukan array".to_string()),
                };

                match heap.get_mut(destination) {
                    Some(Object::Array { elements, .. }) => {
                        let end = destination_pos.saturating_add(length);
                        if end > elements.len() {
                            return Err(format!(
                                "arraycopy: rentang tujuan {destination_pos}..{end} melebihi {} elemen",
                                elements.len()
                            ));
                        }
                        elements[destination_pos..end].copy_from_slice(&copied);
                    }
                    _ => return Err("arraycopy: tujuan bukan array".to_string()),
                }

                Ok(Value::Int(0))
            })()),
            _ => None,
        }
    }

    fn integer_method(
        heap: &mut Heap,
        method: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match (method, signature) {
            ("parseInt", "(Ljava/lang/String;)I") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                text.trim()
                    .parse::<i32>()
                    .map(Value::Int)
                    .map_err(|error| format!("bukan angka int: {error}"))
            })()),
            ("toString", "(I)Ljava/lang/String;") => Some((|| {
                let value = Self::int_arg(args, 0)?;
                Ok(heap.alloc(Object::String(value.to_string())))
            })()),
            ("valueOf", "(I)Ljava/lang/Integer;") => Some((|| {
                let value = Value::Int(Self::int_arg(args, 0)?);
                Ok(heap.alloc(Object::Boxed {
                    type_name: "Ljava/lang/Integer;".to_string(),
                    value,
                }))
            })()),
            ("valueOf", "(Ljava/lang/String;)Ljava/lang/Integer;") => Some((|| {
                let text = Self::string_arg(heap, args, 0)?;
                let parsed = text
                    .trim()
                    .parse::<i32>()
                    .map_err(|error| format!("bukan angka int: {error}"))?;
                Ok(heap.alloc(Object::Boxed {
                    type_name: "Ljava/lang/Integer;".to_string(),
                    value: Value::Int(parsed),
                }))
            })()),
            ("intValue", "()I") => Some(Self::boxed_value(heap, args).and_then(|value| {
                value
                    .as_int()
                    .map(Value::Int)
                    .ok_or_else(|| "nilai boxed bukan int".to_string())
            })),
            ("longValue", "()J") => Some(Self::boxed_value(heap, args).map(|value| {
                Value::from_long(match value {
                    Value::Wide(bits) => bits as i64,
                    Value::Int(number) => i64::from(number),
                    Value::Ref(_) => 0,
                })
            })),
            ("toString", "()Ljava/lang/String;") => Some((|| {
                let value = Self::boxed_value(heap, args)?;
                let text = match value {
                    Value::Int(number) => number.to_string(),
                    Value::Wide(bits) => (bits as i64).to_string(),
                    Value::Ref(_) => return Err("nilai boxed bukan angka".to_string()),
                };
                Ok(heap.alloc(Object::String(text)))
            })()),
            _ => None,
        }
    }
}

impl NativeResolver for AndroidNativeMethods {
    fn invoke_static(
        &self,
        class_descriptor: &str,
        method_name: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match class_descriptor {
            "Ljava/lang/Math;" => Self::math_method(method_name, "(I)I", args),
            _ => None,
        }
    }

    fn invoke_described(
        &self,
        heap: &mut Heap,
        class_descriptor: &str,
        method_name: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match class_descriptor {
            "Ljava/lang/Math;" => Self::math_method(method_name, signature, args),
            "Ljava/lang/String;" => Self::string_method(heap, method_name, signature, args),
            "Ljava/lang/Integer;" => Self::integer_method(heap, method_name, signature, args),
            "Ljava/lang/Long;" => Self::long_method(heap, method_name, signature, args),
            "Ljava/lang/Thread;" => Self::thread_method(heap, method_name, signature, args),
            "Ljava/lang/StringBuilder;" => Self::builder_method(heap, method_name, signature, args),
            "Ljava/lang/Class;" => Self::class_method(heap, method_name, signature, args),
            "Ljava/lang/reflect/Method;" => {
                Self::reflect_method(heap, method_name, signature, args)
            }
            "Ljava/util/ArrayList;"
            | "Ljava/util/LinkedList;"
            | "Ljava/util/AbstractList;"
            | "Ljava/util/List;"
            | "Ljava/util/Collection;"
            | "Ljava/util/Queue;"
            | "Ljava/util/Deque;" => {
                Self::list_method(heap, class_descriptor, method_name, signature, args)
            }
            "Ljava/util/HashMap;"
            | "Ljava/util/LinkedHashMap;"
            | "Ljava/util/concurrent/ConcurrentHashMap;"
            | "Ljava/util/concurrent/ConcurrentMap;"
            | "Ljava/util/Map;"
            | "Ljava/util/AbstractMap;" => {
                Self::map_method(heap, class_descriptor, method_name, signature, args)
            }
            "Ljava/util/Objects;" => Self::objects_method(heap, method_name, signature, args),
            "Ljava/util/Arrays;" => Self::arrays_method(heap, method_name, signature, args),
            "Ljava/util/UUID;" => Self::uuid_method(heap, method_name, signature, args),
            "Ljava/util/Timer;" => Self::timer_method(heap, method_name, signature, args),
            "Landroid/os/Looper;" => Self::looper_method(heap, method_name, signature, args),
            "Landroid/os/Handler;" => Self::handler_method(heap, method_name, signature, args),
            "Ljava/util/concurrent/Executors;" => {
                Self::executors_method(heap, method_name, signature, args)
            }
            "Ljava/util/concurrent/ExecutorService;"
            | "Ljava/util/concurrent/Executor;"
            | "Ljava/util/concurrent/ThreadPoolExecutor;"
            | "Ljava/util/concurrent/ScheduledExecutorService;" => {
                Self::executor_method(heap, method_name, signature, args)
            }
            "Ljava/util/concurrent/Future;" | "Ljava/util/concurrent/ScheduledFuture;" => {
                Self::future_method(heap, method_name, signature, args)
            }
            "Ljava/util/TimerTask;" => Self::timer_task_method(heap, method_name, signature, args),
            "Landroid/os/Process;" => Self::process_method(method_name, signature),
            "Landroid/text/TextUtils;" => {
                Self::text_utils_method(heap, method_name, signature, args)
            }
            "Ljava/net/URLEncoder;" | "Ljava/net/URLDecoder;" => {
                Self::url_method(heap, method_name, signature, args)
            }
            "Ljava/lang/System;" => Self::system_method(heap, method_name, signature, args),
            // Activity dasar disediakan runtime: konstruktor induk aplikasi
            // sering memanggilnya sebelum field miliknya diinisialisasi.
            "Landroid/app/Activity;"
            | "Landroid/content/Context;"
            | "Landroid/app/Application;" => {
                self.activity_method(heap, method_name, signature, args)
            }
            "Landroid/view/WindowManager;" => {
                Self::window_manager_method(heap, method_name, signature)
            }
            "Landroid/view/Display;" => Self::display_method(method_name, signature),
            "Ljava/math/BigInteger;" => {
                Self::big_integer_method(heap, method_name, signature, args)
            }
            "Ljava/security/MessageDigest;" => {
                Self::message_digest_method(heap, method_name, signature, args)
            }
            "Landroid/content/ContentResolver;" => {
                Self::content_resolver_method(method_name, signature)
            }
            "Landroid/content/res/Resources;" => {
                self.resources_method(heap, method_name, signature, args)
            }
            "Ljava/lang/ref/WeakReference;"
            | "Ljava/lang/ref/SoftReference;"
            | "Ljava/lang/ref/Reference;" => {
                Self::weak_reference_method(heap, method_name, signature, args)
            }
            "Landroid/net/LocalSocket;" => {
                Self::local_socket_method(heap, method_name, signature, args)
            }
            "Landroid/net/LocalSocketAddress;" => {
                Self::local_socket_address_method(heap, method_name, signature, args)
            }
            "Ljava/util/concurrent/locks/ReentrantLock;" | "Ljava/util/concurrent/locks/Lock;" => {
                Self::lock_method(heap, method_name, signature, args)
            }
            "Ljava/util/concurrent/locks/Condition;" => {
                Self::condition_method(method_name, signature)
            }
            "Landroid/provider/Settings$Secure;"
            | "Landroid/provider/Settings$System;"
            | "Landroid/provider/Settings$Global;" => {
                self.settings_method(heap, method_name, signature, args)
            }
            "Landroid/net/wifi/WifiManager;" => {
                Self::wifi_manager_method(heap, method_name, signature)
            }
            "Landroid/net/wifi/WifiInfo;" => Self::wifi_info_method(heap, method_name, signature),
            "Landroid/telephony/TelephonyManager;" => {
                Self::telephony_method(heap, method_name, signature)
            }
            "Landroid/content/pm/PackageManager;" => self.pm_method(heap, method_name, signature),
            "Landroid/os/Bundle;" => Self::bundle_method(heap, method_name, signature, args),
            "Landroid/util/Log;" => Self::log_method(heap, method_name, signature, args),
            "Ljava/io/File;" => Self::file_method(heap, method_name, signature, args),
            "Ljava/io/FileInputStream;"
            | "Ljava/io/FileOutputStream;"
            | "Ljava/io/InputStream;"
            | "Ljava/io/BufferedInputStream;" => {
                self.stream_method(heap, method_name, signature, args)
            }
            "Landroid/content/res/AssetManager;" => {
                self.asset_manager_method(heap, method_name, signature, args)
            }
            "Landroid/view/Window;" => self.window_method(heap, method_name, signature, args),
            "Landroid/view/View;"
            | "Landroid/widget/TextView;"
            | "Landroid/widget/Button;"
            | "Landroid/widget/ImageView;" => self.view_method(heap, method_name, signature, args),
            "Landroid/view/ViewGroup;"
            | "Landroid/widget/FrameLayout;"
            | "Landroid/widget/LinearLayout;"
            | "Landroid/widget/RelativeLayout;" => {
                self.view_group_method(heap, method_name, signature, args)
            }
            "Landroid/graphics/Canvas;" => self.canvas_method(heap, method_name, signature, args),
            "Landroid/graphics/Bitmap;" => self.bitmap_method(heap, method_name, signature, args),
            "Landroid/view/InputEvent;" | "Landroid/view/KeyEvent;" => {
                self.key_event_method(heap, class_descriptor, method_name, signature, args)
            }
            "Ljava/io/FileReader;"
            | "Ljava/io/FileWriter;"
            | "Ljava/io/BufferedReader;"
            | "Ljava/io/BufferedWriter;" => {
                self.text_stream_method(heap, class_descriptor, method_name, signature, args)
            }
            "Landroid/content/SharedPreferences;" => {
                Self::prefs_method(heap, method_name, signature, args)
            }
            "Landroid/content/SharedPreferences$Editor;" => {
                Self::editor_method(heap, method_name, signature, args)
            }
            "Ljava/lang/Object;" => Self::object_method(heap, method_name, signature, args),
            "Ljava/lang/Throwable;"
            | "Ljava/lang/Exception;"
            | "Ljava/lang/RuntimeException;"
            | "Ljava/lang/SecurityException;"
            | "Ljava/io/IOException;"
            | "Ljava/io/FileNotFoundException;" => {
                Self::throwable_method(heap, method_name, signature, args)
            }
            _ => None,
        }
    }

    /// Field framework yang tidak pernah ditulis kode aplikasi.
    ///
    /// `PackageInfo`/`ApplicationInfo` di runtime ini adalah objek kosong;
    /// nilainya baru berarti ketika dibaca. Nilai yang tidak diketahui
    /// manifest tetap dilaporkan sebagai nol/null, bukan dikarang.
    fn field_default(
        &self,
        heap: &mut Heap,
        class_descriptor: &str,
        field_name: &str,
    ) -> Option<Value> {
        let manifest = &self.manifest;
        match (class_descriptor, field_name) {
            ("Landroid/content/pm/PackageInfo;", "packageName")
            | ("Landroid/content/pm/ApplicationInfo;", "packageName") => {
                Some(Self::opt_string(heap, manifest.package.as_deref()))
            }
            ("Landroid/content/pm/PackageInfo;", "versionName") => {
                Some(Self::opt_string(heap, manifest.version_name.as_deref()))
            }
            ("Landroid/content/pm/PackageInfo;", "versionCode") => {
                Some(Value::Int(Self::clamp_int(manifest.version_code)))
            }
            ("Landroid/content/pm/ApplicationInfo;", "targetSdkVersion") => {
                Some(Value::Int(manifest.target_sdk.unwrap_or(0)))
            }
            ("Landroid/content/pm/ApplicationInfo;", "minSdkVersion") => {
                Some(Value::Int(manifest.min_sdk.unwrap_or(0)))
            }
            // `metaData` selalu ada di Android; di sini isinya persis
            // `<meta-data>` yang terbaca dari manifest.
            ("Landroid/content/pm/ApplicationInfo;", "metaData") => {
                let mut entries = Vec::with_capacity(manifest.meta_data.len());
                for (name, value) in &manifest.meta_data {
                    let key = heap.alloc(Object::String(name.clone()));
                    let text = heap.alloc(Object::String(value.clone()));
                    entries.push((key, text));
                }
                Some(heap.alloc(Object::Map {
                    type_name: "Landroid/os/Bundle;".to_string(),
                    entries,
                }))
            }
            // Layar virtual: nilai ini tetap sampai runtime punya permukaan
            // gambar sungguhan.
            ("Landroid/util/DisplayMetrics;", "widthPixels") => Some(Value::Int(VIRTUAL_WIDTH)),
            ("Landroid/util/DisplayMetrics;", "heightPixels") => Some(Value::Int(VIRTUAL_HEIGHT)),
            ("Landroid/util/DisplayMetrics;", "densityDpi") => Some(Value::Int(VIRTUAL_DPI)),
            ("Landroid/util/DisplayMetrics;", "density") => {
                Some(Value::from_float(VIRTUAL_DPI as f32 / 160.0))
            }
            ("Landroid/util/DisplayMetrics;", "scaledDensity") => {
                Some(Value::from_float(VIRTUAL_DPI as f32 / 160.0))
            }
            ("Landroid/util/DisplayMetrics;", "xdpi")
            | ("Landroid/util/DisplayMetrics;", "ydpi") => {
                Some(Value::from_float(VIRTUAL_DPI as f32))
            }
            // Konstanta `KeyEvent` (aksi dan kode tombol) yang sering dibaca
            // aplikasi lewat field statis.
            ("Landroid/view/KeyEvent;", "ACTION_DOWN") => Some(Value::Int(0)),
            ("Landroid/view/KeyEvent;", "ACTION_UP") => Some(Value::Int(1)),
            ("Landroid/view/KeyEvent;", "ACTION_MULTIPLE") => Some(Value::Int(2)),
            ("Landroid/view/KeyEvent;", "KEYCODE_UNKNOWN") => Some(Value::Int(0)),
            ("Landroid/view/KeyEvent;", "KEYCODE_SOFT_LEFT") => Some(Value::Int(1)),
            ("Landroid/view/KeyEvent;", "KEYCODE_SOFT_RIGHT") => Some(Value::Int(2)),
            ("Landroid/view/KeyEvent;", "KEYCODE_HOME") => Some(Value::Int(3)),
            ("Landroid/view/KeyEvent;", "KEYCODE_BACK") => Some(Value::Int(4)),
            ("Landroid/view/KeyEvent;", "KEYCODE_DPAD_UP") => Some(Value::Int(19)),
            ("Landroid/view/KeyEvent;", "KEYCODE_DPAD_DOWN") => Some(Value::Int(20)),
            ("Landroid/view/KeyEvent;", "KEYCODE_DPAD_LEFT") => Some(Value::Int(21)),
            ("Landroid/view/KeyEvent;", "KEYCODE_DPAD_RIGHT") => Some(Value::Int(22)),
            ("Landroid/view/KeyEvent;", "KEYCODE_DPAD_CENTER") => Some(Value::Int(23)),
            ("Landroid/view/KeyEvent;", "KEYCODE_ENTER") => Some(Value::Int(66)),
            ("Landroid/view/KeyEvent;", "KEYCODE_DEL") => Some(Value::Int(67)),
            ("Landroid/view/KeyEvent;", "KEYCODE_MENU") => Some(Value::Int(82)),
            ("Landroid/view/KeyEvent;", "KEYCODE_A") => Some(Value::Int(29)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_utils_menganggap_null_sebagai_kosong() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let empty = heap.alloc(Object::String(String::new()));
        let filled = heap.alloc(Object::String("isi".to_owned()));

        let check = |heap: &mut Heap, value: Value| {
            resolver
                .invoke_described(
                    heap,
                    "Landroid/text/TextUtils;",
                    "isEmpty",
                    "(Ljava/lang/CharSequence;)Z",
                    &[value],
                )
                .expect("isEmpty harus tersedia")
                .expect("isEmpty harus berhasil")
        };

        assert_eq!(check(&mut heap, Value::Int(0)), Value::Int(1));
        assert_eq!(check(&mut heap, empty), Value::Int(1));
        assert_eq!(check(&mut heap, filled), Value::Int(0));
    }

    #[test]
    fn string_pencocokan_awalan_akhiran_dan_trim() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let text = heap.alloc(Object::String("  berkas.apk  ".to_owned()));
        let prefix = heap.alloc(Object::String("  berkas".to_owned()));
        let suffix = heap.alloc(Object::String(".apk  ".to_owned()));

        let check = |heap: &mut Heap, method: &str, signature: &str, extra: &[Value]| {
            let mut args = vec![text];
            args.extend_from_slice(extra);
            resolver
                .invoke_described(heap, "Ljava/lang/String;", method, signature, &args)
                .expect("method harus tersedia")
                .expect("method harus berhasil")
        };

        assert_eq!(
            check(&mut heap, "startsWith", "(Ljava/lang/String;)Z", &[prefix]),
            Value::Int(1)
        );
        assert_eq!(
            check(&mut heap, "endsWith", "(Ljava/lang/String;)Z", &[suffix]),
            Value::Int(1)
        );

        let trimmed = check(&mut heap, "trim", "()Ljava/lang/String;", &[]);
        let reference = trimmed.as_ref().expect("trim harus String");
        assert_eq!(heap.string(reference), Some("berkas.apk"));
    }

    #[test]
    fn class_for_name_mengubah_nama_java_menjadi_descriptor() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let name = heap.alloc(Object::String("java.lang.String".to_owned()));
        let class = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/Class;",
                "forName",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[name],
            )
            .expect("forName harus tersedia")
            .expect("forName harus berhasil");
        let reference = class.as_ref().expect("harus Class");
        match heap.get(reference) {
            Some(Object::Class { descriptor }) => {
                assert_eq!(descriptor, "Ljava/lang/String;");
            }
            other => panic!("bukan Class: {other:?}"),
        }
    }

    #[test]
    fn class_get_canonical_name_memakai_nama_java() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let class = heap.alloc(Object::Class {
            descriptor: "Ljava/util/UUID;".to_string(),
        });
        let name = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/Class;",
                "getCanonicalName",
                "()Ljava/lang/String;",
                &[class],
            )
            .expect("getCanonicalName harus tersedia")
            .expect("getCanonicalName harus berhasil");
        let reference = name.as_ref().expect("harus String");
        assert_eq!(heap.string(reference), Some("java.util.UUID"));
    }

    #[test]
    fn get_application_mengembalikan_objek_application() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let application = resolver
            .invoke_described(
                &mut heap,
                "Landroid/app/Activity;",
                "getApplication",
                "()Landroid/app/Application;",
                &[Value::Int(0)],
            )
            .expect("getApplication harus tersedia")
            .expect("getApplication harus berhasil");
        let reference = application.as_ref().expect("harus referensi");
        match heap.get(reference) {
            Some(Object::Instance { type_name, .. }) => {
                assert_eq!(type_name, "Landroid/app/Application;");
            }
            other => panic!("bukan instance: {other:?}"),
        }
    }

    #[test]
    fn thread_tanpa_handler_eksepsi_global() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let handler = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/Thread;",
                "getDefaultUncaughtExceptionHandler",
                "()Ljava/lang/Thread$UncaughtExceptionHandler;",
                &[],
            )
            .expect("getDefaultUncaughtExceptionHandler harus tersedia")
            .expect("getDefaultUncaughtExceptionHandler harus berhasil");
        // Harus mengembalikan objek Thread$UncaughtExceptionHandler, bukan null.
        assert!(matches!(handler, Value::Ref(_)));
    }

    #[test]
    fn thread_utama_bernama_main() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let thread = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/Thread;",
                "currentThread",
                "()Ljava/lang/Thread;",
                &[],
            )
            .expect("currentThread harus tersedia")
            .expect("currentThread harus berhasil");
        let name = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/Thread;",
                "getName",
                "()Ljava/lang/String;",
                &[thread],
            )
            .expect("getName harus tersedia")
            .expect("getName harus berhasil");
        let reference = name.as_ref().expect("getName harus String");
        assert_eq!(heap.string(reference), Some("main"));
    }

    #[test]
    fn long_parse_dan_boxing_bolak_balik() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let text = heap.alloc(Object::String("9000000000".to_owned()));
        let parsed = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/Long;",
                "parseLong",
                "(Ljava/lang/String;)J",
                &[text],
            )
            .expect("parseLong harus tersedia")
            .expect("parseLong harus berhasil");
        assert_eq!(parsed.as_long(), Some(9_000_000_000));

        let boxed = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/Long;",
                "valueOf",
                "(J)Ljava/lang/Long;",
                &[parsed],
            )
            .expect("valueOf harus tersedia")
            .expect("valueOf harus berhasil");
        let unboxed = resolver
            .invoke_described(&mut heap, "Ljava/lang/Long;", "longValue", "()J", &[boxed])
            .expect("longValue harus tersedia")
            .expect("longValue harus berhasil");
        assert_eq!(unboxed.as_long(), Some(9_000_000_000));
    }

    #[test]
    fn process_my_pid_mengembalikan_pid_positif() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let pid = resolver
            .invoke_described(&mut heap, "Landroid/os/Process;", "myPid", "()I", &[])
            .expect("myPid harus tersedia")
            .expect("myPid harus berhasil");
        assert!(pid.as_int().unwrap_or(0) > 0);
    }

    #[test]
    fn konstruktor_string_dari_byte_membaca_utf8() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let bytes = heap.alloc(Object::Array {
            element_type: "B".to_string(),
            elements: vec![Value::Int(0x68), Value::Int(0x69)],
        });
        let instance = heap.alloc(Object::Instance {
            type_name: "Ljava/lang/String;".to_string(),
            fields: std::collections::HashMap::new(),
        });
        resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/String;",
                "<init>",
                "([B)V",
                &[instance, bytes],
            )
            .expect("<init> harus tersedia")
            .expect("<init> harus berhasil");
        let reference = instance.as_ref().expect("instance harus referensi");
        assert_eq!(heap.string(reference), Some("hi"));
    }

    #[test]
    fn url_encoder_dan_decoder_bolak_balik() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let source = heap.alloc(Object::String("a b&c".to_owned()));

        let encoded = resolver
            .invoke_described(
                &mut heap,
                "Ljava/net/URLEncoder;",
                "encode",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[source],
            )
            .expect("encode harus tersedia")
            .expect("encode harus berhasil");
        let encoded_ref = encoded.as_ref().expect("encode harus String");
        assert_eq!(heap.string(encoded_ref), Some("a+b%26c"));

        let decoded = resolver
            .invoke_described(
                &mut heap,
                "Ljava/net/URLDecoder;",
                "decode",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[encoded],
            )
            .expect("decode harus tersedia")
            .expect("decode harus berhasil");
        let decoded_ref = decoded.as_ref().expect("decode harus String");
        assert_eq!(heap.string(decoded_ref), Some("a b&c"));
    }

    #[test]
    fn url_decoder_menolak_escape_tidak_lengkap() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let broken = heap.alloc(Object::String("abc%2".to_owned()));
        let outcome = resolver
            .invoke_described(
                &mut heap,
                "Ljava/net/URLDecoder;",
                "decode",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[broken],
            )
            .expect("decode harus tersedia");
        assert!(outcome.is_err());
    }

    #[test]
    fn string_get_bytes_memakai_utf8() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let text = heap.alloc(Object::String("é".to_owned()));
        let bytes = resolver
            .invoke_described(&mut heap, "Ljava/lang/String;", "getBytes", "()[B", &[text])
            .expect("getBytes harus tersedia")
            .expect("getBytes harus berhasil");
        let reference = bytes.as_ref().expect("getBytes harus array");
        match heap.get(reference) {
            Some(Object::Array {
                element_type,
                elements,
            }) => {
                assert_eq!(element_type, "B");
                let values: Vec<i32> = elements.iter().filter_map(|value| value.as_int()).collect();
                assert_eq!(values, vec![0xC3, 0xA9]);
            }
            other => panic!("bukan array: {other:?}"),
        }
    }

    #[test]
    fn string_content_equals_membandingkan_isi() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let left = heap.alloc(Object::String("halo".to_owned()));
        let same = heap.alloc(Object::String("halo".to_owned()));
        let other = heap.alloc(Object::String("dunia".to_owned()));

        let check = |heap: &mut Heap, right: Value| {
            resolver
                .invoke_described(
                    heap,
                    "Ljava/lang/String;",
                    "contentEquals",
                    "(Ljava/lang/CharSequence;)Z",
                    &[left, right],
                )
                .expect("contentEquals harus tersedia")
                .expect("contentEquals harus berhasil")
        };

        assert_eq!(check(&mut heap, same), Value::Int(1));
        assert_eq!(check(&mut heap, other), Value::Int(0));
    }

    #[test]
    fn random_uuid_membentuk_teks_versi_4() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let uuid = resolver
            .invoke_described(
                &mut heap,
                "Ljava/util/UUID;",
                "randomUUID",
                "()Ljava/util/UUID;",
                &[],
            )
            .expect("randomUUID harus tersedia")
            .expect("randomUUID harus berhasil");
        let text = resolver
            .invoke_described(
                &mut heap,
                "Ljava/util/UUID;",
                "toString",
                "()Ljava/lang/String;",
                &[uuid],
            )
            .expect("toString harus tersedia")
            .expect("toString harus berhasil");
        let reference = text.as_ref().expect("toString harus String");
        let text = heap.string(reference).expect("String UUID").to_string();

        assert_eq!(text.len(), 36);
        let bytes = text.as_bytes();
        for dash in [8usize, 13, 18, 23] {
            assert_eq!(bytes[dash], b'-', "tanda hubung di {dash}: {text}");
        }
        assert_eq!(bytes[14], b'4', "versi UUID harus 4: {text}");
        assert!(
            matches!(bytes[19], b'8' | b'9' | b'a' | b'b'),
            "varian RFC 4122: {text}"
        );
    }

    #[test]
    fn shared_preferences_menyimpan_nilai_di_memori() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let key = heap.alloc(Object::String("kunci".to_owned()));
        let prefs = resolver
            .invoke_described(
                &mut heap,
                "Landroid/content/Context;",
                "getSharedPreferences",
                "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
                &[Value::Int(0), key, Value::Int(0)],
            )
            .expect("getSharedPreferences harus tersedia")
            .expect("getSharedPreferences harus berhasil");

        let again = resolver
            .invoke_described(
                &mut heap,
                "Landroid/content/Context;",
                "getSharedPreferences",
                "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
                &[Value::Int(0), key, Value::Int(0)],
            )
            .expect("getSharedPreferences harus tersedia")
            .expect("getSharedPreferences harus berhasil");
        assert_eq!(prefs, again, "preferensi dengan nama sama harus sama");

        resolver
            .invoke_described(
                &mut heap,
                "Landroid/content/SharedPreferences$Editor;",
                "putInt",
                "(Ljava/lang/String;I)Landroid/content/SharedPreferences$Editor;",
                &[prefs, key, Value::Int(7)],
            )
            .expect("putInt harus tersedia")
            .expect("putInt harus berhasil");

        let stored = resolver
            .invoke_described(
                &mut heap,
                "Landroid/content/SharedPreferences;",
                "getInt",
                "(Ljava/lang/String;I)I",
                &[prefs, key, Value::Int(0)],
            )
            .expect("getInt harus tersedia")
            .expect("getInt harus berhasil");
        assert_eq!(stored, Value::Int(7));
    }

    #[test]
    fn file_dari_get_files_dir_memiliki_path_absolut() {
        let resolver = AndroidNativeMethods::with_app_dir("/data/uji".into());
        let mut heap = Heap::new();
        let file = resolver
            .invoke_described(
                &mut heap,
                "Landroid/content/Context;",
                "getFilesDir",
                "()Ljava/io/File;",
                &[Value::Int(0)],
            )
            .expect("getFilesDir harus tersedia")
            .expect("getFilesDir harus berhasil");

        let absolute = resolver
            .invoke_described(
                &mut heap,
                "Ljava/io/File;",
                "getAbsolutePath",
                "()Ljava/lang/String;",
                &[file],
            )
            .expect("getAbsolutePath harus tersedia")
            .expect("getAbsolutePath harus berhasil");
        let text = absolute
            .as_ref()
            .and_then(|reference| heap.string(reference))
            .expect("hasil harus String");
        assert_eq!(text, "/data/uji/files");
    }

    #[test]
    fn log_mengembalikan_panjang_pesan() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let tag = heap.alloc(Object::String("Tag".to_owned()));
        let message = heap.alloc(Object::String("halo".to_owned()));
        let result = resolver
            .invoke_described(
                &mut heap,
                "Landroid/util/Log;",
                "d",
                "(Ljava/lang/String;Ljava/lang/String;)I",
                &[tag, message],
            )
            .expect("Log.d harus tersedia");
        assert_eq!(result, Ok(Value::Int(4)));
    }

    #[test]
    fn activity_lifecycle_dasar_diterima_tanpa_efek() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        assert_eq!(
            resolver.invoke_described(
                &mut heap,
                "Landroid/app/Activity;",
                "onCreate",
                "(Landroid/os/Bundle;)V",
                &[Value::Int(0)],
            ),
            Some(Ok(Value::Int(0)))
        );
        // Konfigurasi jendela diterima sebagai no-op tanpa klaim tampilan.
        assert_eq!(
            resolver.invoke_described(
                &mut heap,
                "Landroid/app/Activity;",
                "setContentView",
                "(I)V",
                &[Value::Int(0)],
            ),
            Some(Ok(Value::Int(0)))
        );
        // Method yang belum didukung tetap dilaporkan tidak tersedia.
        assert_eq!(
            resolver.invoke_described(
                &mut heap,
                "Landroid/app/Activity;",
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[Value::Int(0), Value::Int(0)],
            ),
            None
        );
    }

    #[test]
    fn math_abs_mengembalikan_nilai_mutlak() {
        let resolver = AndroidNativeMethods::new();
        let result = resolver.invoke_static("Ljava/lang/Math;", "abs", &[Value::Int(-9)]);
        assert_eq!(result, Some(Ok(Value::Int(9))));
    }

    #[test]
    fn method_tidak_dikenal_dilaporkan_none() {
        let resolver = AndroidNativeMethods::new();
        assert_eq!(
            resolver.invoke_static("Ljava/lang/System;", "exit", &[]),
            None
        );
    }

    #[test]
    fn string_length_menghitung_satuan_utf16() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let text = heap.alloc(Object::String("halo".to_owned()));
        let result =
            resolver.invoke_described(&mut heap, "Ljava/lang/String;", "length", "()I", &[text]);
        assert_eq!(result, Some(Ok(Value::Int(4))));
    }

    #[test]
    fn string_char_at_mengembalikan_satuan_utf16() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let text = heap.alloc(Object::String("halo".to_owned()));
        let result = resolver.invoke_described(
            &mut heap,
            "Ljava/lang/String;",
            "charAt",
            "(I)C",
            &[text, Value::Int(1)],
        );
        assert_eq!(result, Some(Ok(Value::Int('a' as i32))));
    }

    #[test]
    fn integer_parse_int_menolak_teks_bukan_angka() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let text = heap.alloc(Object::String("bukan angka".to_owned()));
        let result = resolver.invoke_described(
            &mut heap,
            "Ljava/lang/Integer;",
            "parseInt",
            "(Ljava/lang/String;)I",
            &[text],
        );
        assert!(matches!(result, Some(Err(_))));
    }

    #[test]
    fn argumen_bukan_int_dilaporkan_sebagai_error() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let text = heap.alloc(Object::String("teks".to_owned()));
        let result = resolver.invoke_static("Ljava/lang/Math;", "abs", &[text]);
        assert!(matches!(result, Some(Err(_))));
    }

    #[test]
    fn thread_to_string_memakai_nama_dan_prioritas() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let thread = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/Thread;",
                "currentThread",
                "()Ljava/lang/Thread;",
                &[],
            )
            .expect("currentThread harus tersedia")
            .expect("currentThread harus berhasil");

        let text = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/Thread;",
                "toString",
                "()Ljava/lang/String;",
                &[thread],
            )
            .expect("toString harus tersedia")
            .expect("toString harus berhasil");

        let reference = text.as_ref().expect("toString mengembalikan objek");
        assert_eq!(heap.string(reference), Some("Thread[main,5,main]"));
    }

    #[test]
    fn log_get_stack_trace_string_memuat_kelas_dan_pesan() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let message = heap.alloc(Object::String("berkas hilang".to_owned()));
        let mut fields = std::collections::HashMap::new();
        fields.insert(0, message);
        let throwable = heap.alloc(Object::Instance {
            type_name: "Ljava/io/FileNotFoundException;".to_owned(),
            fields,
        });

        let text = resolver
            .invoke_described(
                &mut heap,
                "Landroid/util/Log;",
                "getStackTraceString",
                "(Ljava/lang/Throwable;)Ljava/lang/String;",
                &[throwable],
            )
            .expect("getStackTraceString harus tersedia")
            .expect("getStackTraceString harus berhasil");

        let reference = text.as_ref().expect("hasil berupa objek");
        assert_eq!(
            heap.string(reference),
            Some("java.io.FileNotFoundException: berkas hilang")
        );
    }

    #[test]
    fn string_builder_append_object_menangani_null_dan_boxed() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let builder = heap
            .alloc(Object::Instance {
                type_name: "Ljava/lang/StringBuilder;".to_owned(),
                fields: std::collections::HashMap::new(),
            })
            .as_ref()
            .expect("builder berada di heap");
        let initial = heap.alloc(Object::String(String::new()));
        if let Some(Object::Instance { fields, .. }) = heap.get_mut(builder) {
            fields.insert(0, initial);
        }

        let append = |heap: &mut Heap, value: Value| {
            resolver
                .invoke_described(
                    heap,
                    "Ljava/lang/StringBuilder;",
                    "append",
                    "(Ljava/lang/Object;)Ljava/lang/StringBuilder;",
                    &[Value::Ref(builder), value],
                )
                .expect("append harus tersedia")
                .expect("append harus berhasil")
        };

        let text = heap.alloc(Object::String("nilai=".to_owned()));
        append(&mut heap, text);
        let boxed = heap.alloc(Object::Boxed {
            type_name: "Ljava/lang/Boolean;".to_owned(),
            value: Value::Int(1),
        });
        append(&mut heap, boxed);
        append(&mut heap, Value::Int(0));

        let result = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/StringBuilder;",
                "toString",
                "()Ljava/lang/String;",
                &[Value::Ref(builder)],
            )
            .expect("toString harus tersedia")
            .expect("toString harus berhasil");
        let reference = result.as_ref().expect("hasil berupa objek");
        assert_eq!(heap.string(reference), Some("nilai=truenull"));
    }

    #[test]
    fn string_builder_append_float_memakai_bentuk_java() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let builder = heap
            .alloc(Object::Instance {
                type_name: "Ljava/lang/StringBuilder;".to_owned(),
                fields: std::collections::HashMap::new(),
            })
            .as_ref()
            .expect("builder berada di heap");
        let initial = heap.alloc(Object::String(String::new()));
        if let Some(Object::Instance { fields, .. }) = heap.get_mut(builder) {
            fields.insert(0, initial);
        }
        resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/StringBuilder;",
                "append",
                "(F)Ljava/lang/StringBuilder;",
                &[Value::Ref(builder), Value::from_float(1.5)],
            )
            .expect("append(F) harus tersedia")
            .expect("append(F) harus berhasil");
        let result = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/StringBuilder;",
                "toString",
                "()Ljava/lang/String;",
                &[Value::Ref(builder)],
            )
            .expect("toString harus tersedia")
            .expect("toString harus berhasil");
        let reference = result.as_ref().expect("hasil berupa objek");
        assert_eq!(heap.string(reference), Some("1.5"));
    }

    #[test]
    fn izin_manifest_diberikan_dan_izin_lain_ditolak() {
        let resolver = AndroidNativeMethods::new()
            .with_permissions(vec!["android.permission.INTERNET".to_string()]);
        let mut heap = Heap::new();
        let declared = heap.alloc(Object::String("android.permission.INTERNET".to_owned()));
        let unknown = heap.alloc(Object::String("android.permission.CAMERA".to_owned()));

        let check = |heap: &mut Heap, name: Value| {
            resolver
                .invoke_described(
                    heap,
                    "Landroid/content/Context;",
                    "checkCallingOrSelfPermission",
                    "(Ljava/lang/String;)I",
                    &[Value::Int(0), name],
                )
                .expect("checkCallingOrSelfPermission harus tersedia")
                .expect("checkCallingOrSelfPermission harus berhasil")
        };

        assert_eq!(check(&mut heap, declared), Value::Int(0));
        assert_eq!(check(&mut heap, unknown), Value::Int(-1));
    }

    #[test]
    fn layanan_sistem_window_menyediakan_display_virtual() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let name = heap.alloc(Object::String("window".to_owned()));

        let service = resolver
            .invoke_described(
                &mut heap,
                "Landroid/content/Context;",
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[Value::Int(0), name],
            )
            .expect("getSystemService harus tersedia")
            .expect("getSystemService harus berhasil");
        let reference = service.as_ref().expect("layanan berupa objek");
        assert!(matches!(
            heap.get(reference),
            Some(Object::Instance { type_name, .. }) if type_name == "Landroid/view/WindowManager;"
        ));

        let display = resolver
            .invoke_described(
                &mut heap,
                "Landroid/view/WindowManager;",
                "getDefaultDisplay",
                "()Landroid/view/Display;",
                &[service],
            )
            .expect("getDefaultDisplay harus tersedia")
            .expect("getDefaultDisplay harus berhasil");
        let width = resolver
            .invoke_described(
                &mut heap,
                "Landroid/view/Display;",
                "getWidth",
                "()I",
                &[display],
            )
            .expect("getWidth harus tersedia")
            .expect("getWidth harus berhasil");
        assert_eq!(width, Value::Int(VIRTUAL_WIDTH));
        assert_eq!(
            resolver.field_default(&mut heap, "Landroid/util/DisplayMetrics;", "widthPixels"),
            Some(Value::Int(VIRTUAL_WIDTH))
        );

        let unknown = heap.alloc(Object::String("tidak-ada".to_owned()));
        let missing = resolver
            .invoke_described(
                &mut heap,
                "Landroid/content/Context;",
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[Value::Int(0), unknown],
            )
            .expect("getSystemService harus tersedia")
            .expect("getSystemService harus berhasil");
        assert_eq!(missing, Value::Int(0));
    }

    #[test]
    fn wifi_info_melaporkan_mac_placeholder() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let info = resolver
            .invoke_described(
                &mut heap,
                "Landroid/net/wifi/WifiManager;",
                "getConnectionInfo",
                "()Landroid/net/wifi/WifiInfo;",
                &[Value::Int(0)],
            )
            .expect("getConnectionInfo harus tersedia")
            .expect("getConnectionInfo harus berhasil");
        let mac = resolver
            .invoke_described(
                &mut heap,
                "Landroid/net/wifi/WifiInfo;",
                "getMacAddress",
                "()Ljava/lang/String;",
                &[info],
            )
            .expect("getMacAddress harus tersedia")
            .expect("getMacAddress harus berhasil");
        let reference = mac.as_ref().expect("MAC berupa objek");
        assert_eq!(heap.string(reference), Some(VIRTUAL_MAC));

        let enabled = resolver
            .invoke_described(
                &mut heap,
                "Landroid/net/wifi/WifiManager;",
                "isWifiEnabled",
                "()Z",
                &[Value::Int(0)],
            )
            .expect("isWifiEnabled harus tersedia")
            .expect("isWifiEnabled harus berhasil");
        assert_eq!(enabled, Value::Int(0));
    }

    #[test]
    fn replace_all_memakai_regex_dan_menolak_lookbehind() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let text = heap.alloc(Object::String("a-b-c".to_owned()));
        let pattern = heap.alloc(Object::String("-".to_owned()));
        let replacement = heap.alloc(Object::String("/".to_owned()));
        let result = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/String;",
                "replaceAll",
                "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                &[text, pattern, replacement],
            )
            .expect("replaceAll harus tersedia")
            .expect("replaceAll harus berhasil");
        let reference = result.as_ref().expect("hasil berupa objek");
        assert_eq!(heap.string(reference), Some("a/b/c"));

        let spaced = heap.alloc(Object::String("a   b".to_owned()));
        let whitespace = heap.alloc(Object::String("\\s+".to_owned()));
        let dashed = heap.alloc(Object::String("-".to_owned()));
        let collapsed = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/String;",
                "replaceAll",
                "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                &[spaced, whitespace, dashed],
            )
            .expect("replaceAll regex harus tersedia")
            .expect("replaceAll regex harus berhasil");
        let collapsed_ref = collapsed.as_ref().expect("hasil berupa objek");
        assert_eq!(heap.string(collapsed_ref), Some("a-b"));

        // Lookbehind tidak didukung mesin regex Rust dan harus dilaporkan.
        let lookbehind = heap.alloc(Object::String("(?<=a)b".to_owned()));
        let rejected = resolver.invoke_described(
            &mut heap,
            "Ljava/lang/String;",
            "replaceAll",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            &[text, lookbehind, replacement],
        );
        assert!(matches!(rejected, Some(Err(_))));
    }

    #[test]
    fn big_integer_menghitung_di_luar_jangkauan_long() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let call = |heap: &mut Heap, method: &str, signature: &str, args: &[Value]| {
            resolver
                .invoke_described(heap, "Ljava/math/BigInteger;", method, signature, args)
                .expect("method BigInteger harus tersedia")
                .expect("method BigInteger harus berhasil")
        };

        let text = heap.alloc(Object::String("123456789012345678901234567890".to_owned()));
        let value = heap.alloc(Object::Instance {
            type_name: "Ljava/math/BigInteger;".to_owned(),
            fields: std::collections::HashMap::new(),
        });
        call(&mut heap, "<init>", "(Ljava/lang/String;)V", &[value, text]);
        let one = call(
            &mut heap,
            "valueOf",
            "(J)Ljava/math/BigInteger;",
            &[Value::from_long(1)],
        );
        let sum = call(
            &mut heap,
            "add",
            "(Ljava/math/BigInteger;)Ljava/math/BigInteger;",
            &[value, one],
        );
        let rendered = call(&mut heap, "toString", "()Ljava/lang/String;", &[sum]);
        let reference = rendered.as_ref().expect("toString berupa objek");
        assert_eq!(
            heap.string(reference),
            Some("123456789012345678901234567891")
        );

        // Konstruktor signum + magnitude: 0x01 0x00 berarti 256.
        let magnitude = heap.alloc(Object::Array {
            element_type: "B".to_owned(),
            elements: vec![Value::Int(1), Value::Int(0)],
        });
        let from_bytes = heap.alloc(Object::Instance {
            type_name: "Ljava/math/BigInteger;".to_owned(),
            fields: std::collections::HashMap::new(),
        });
        call(
            &mut heap,
            "<init>",
            "(I[B)V",
            &[from_bytes, Value::Int(1), magnitude],
        );
        let text = call(&mut heap, "toString", "()Ljava/lang/String;", &[from_bytes]);
        let reference = text.as_ref().expect("toString berupa objek");
        assert_eq!(heap.string(reference), Some("256"));
    }

    #[test]
    fn string_format_menangani_penanda_umum() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let pattern = heap.alloc(Object::String("%s:%d:%.2f".to_owned()));
        let text = heap.alloc(Object::String("a".to_owned()));
        let array = heap.alloc(Object::Array {
            element_type: "Ljava/lang/Object;".to_owned(),
            elements: vec![text, Value::Int(7), Value::from_double(1.5)],
        });
        let result = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/String;",
                "format",
                "(Ljava/lang/String;[Ljava/lang/Object;)Ljava/lang/String;",
                &[pattern, array],
            )
            .expect("format harus tersedia")
            .expect("format harus berhasil");
        let reference = result.as_ref().expect("hasil berupa objek");
        assert_eq!(heap.string(reference), Some("a:7:1.50"));

        let padded_pattern = heap.alloc(Object::String("%04d".to_owned()));
        let padded_array = heap.alloc(Object::Array {
            element_type: "Ljava/lang/Object;".to_owned(),
            elements: vec![Value::Int(42)],
        });
        let padded = resolver
            .invoke_described(
                &mut heap,
                "Ljava/lang/String;",
                "format",
                "(Ljava/lang/String;[Ljava/lang/Object;)Ljava/lang/String;",
                &[padded_pattern, padded_array],
            )
            .expect("format harus tersedia")
            .expect("format harus berhasil");
        let reference = padded.as_ref().expect("hasil berupa objek");
        assert_eq!(heap.string(reference), Some("0042"));
    }

    #[test]
    fn settings_secure_memberi_android_id_stabil_per_paket() {
        let resolver = AndroidNativeMethods::new().with_package("com.contoh");
        let mut heap = Heap::new();
        let name = heap.alloc(Object::String("android_id".to_owned()));
        let read = |heap: &mut Heap, key: Value| {
            resolver
                .invoke_described(
                    heap,
                    "Landroid/provider/Settings$Secure;",
                    "getString",
                    "(Landroid/content/ContentResolver;Ljava/lang/String;)Ljava/lang/String;",
                    &[Value::Int(0), key],
                )
                .expect("getString harus tersedia")
                .expect("getString harus berhasil")
        };
        let first = read(&mut heap, name);
        let second = read(&mut heap, name);
        let reference = first.as_ref().expect("android_id berupa objek");
        let text = heap
            .string(reference)
            .expect("android_id berupa teks")
            .to_owned();
        assert_eq!(text.len(), 16);
        assert!(text.chars().all(|character| character.is_ascii_hexdigit()));
        let second_ref = second.as_ref().expect("android_id berupa objek");
        assert_eq!(heap.string(second_ref), Some(text.as_str()));

        let other = heap.alloc(Object::String("kunci_lain".to_owned()));
        assert_eq!(read(&mut heap, other), Value::Int(0));
    }
    #[test]
    fn message_digest_menghitung_md5_dan_sha256() {
        let resolver = AndroidNativeMethods::new();
        let mut heap = Heap::new();
        let digest_of = |heap: &mut Heap, algorithm: &str, input: &str| {
            let name = heap.alloc(Object::String(algorithm.to_owned()));
            let digest = resolver
                .invoke_described(
                    heap,
                    "Ljava/security/MessageDigest;",
                    "getInstance",
                    "(Ljava/lang/String;)Ljava/security/MessageDigest;",
                    &[name],
                )
                .expect("getInstance harus tersedia")
                .expect("getInstance harus berhasil");
            let bytes: Vec<Value> = input
                .bytes()
                .map(|byte| Value::Int(i32::from(byte)))
                .collect();
            let data = heap.alloc(Object::Array {
                element_type: "B".to_owned(),
                elements: bytes,
            });
            resolver
                .invoke_described(
                    heap,
                    "Ljava/security/MessageDigest;",
                    "update",
                    "([B)V",
                    &[digest, data],
                )
                .expect("update harus tersedia")
                .expect("update harus berhasil");
            let result = resolver
                .invoke_described(
                    heap,
                    "Ljava/security/MessageDigest;",
                    "digest",
                    "()[B",
                    &[digest],
                )
                .expect("digest harus tersedia")
                .expect("digest harus berhasil");
            let reference = result.as_ref().expect("digest berupa array");
            match heap.get(reference) {
                Some(Object::Array { elements, .. }) => elements
                    .iter()
                    .map(|value| format!("{:02x}", value.as_int().unwrap_or(0)))
                    .collect::<String>(),
                _ => panic!("digest bukan array"),
            }
        };

        assert_eq!(
            digest_of(&mut heap, "MD5", "abc"),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            digest_of(&mut heap, "SHA-256", "abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let unknown = heap.alloc(Object::String("TIDAK-ADA".to_owned()));
        let rejected = resolver.invoke_described(
            &mut heap,
            "Ljava/security/MessageDigest;",
            "getInstance",
            "(Ljava/lang/String;)Ljava/security/MessageDigest;",
            &[unknown],
        );
        assert!(matches!(rejected, Some(Err(_))));
    }
}
