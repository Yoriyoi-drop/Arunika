//! Fixture APK sintetis untuk pengujian Arunika.
//!
//! Crate ini hanya dipakai sebagai dev-dependency. Tujuannya menghasilkan
//! APK/XAPK/APKM minimal yang valid menurut parser AXML dan ZIP, sehingga tes
//! tidak bergantung pada berkas biner besar yang tidak ikut berversi.

use std::collections::HashMap;
use std::io::{self, Cursor, Write};
use std::path::Path;

pub mod dex;
pub use dex::{ClassSpec, MethodSpec, build_dex};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const ANDROID_NS: &str = "http://schemas.android.com/apk/res/android";
const ANDROID_PREFIX: &str = "android";

const CHUNK_XML: u16 = 0x0003;
const CHUNK_STRING_POOL: u16 = 0x0001;
const CHUNK_RESOURCE_MAP: u16 = 0x0180;
const CHUNK_START_NAMESPACE: u16 = 0x0100;
const CHUNK_END_NAMESPACE: u16 = 0x0101;
const CHUNK_START_ELEMENT: u16 = 0x0102;
const CHUNK_END_ELEMENT: u16 = 0x0103;

const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_BOOLEAN: u8 = 0x12;

const NO_INDEX: u32 = u32::MAX;
const UTF8_FLAG: u32 = 1 << 8;

/// Deskripsi aplikasi sintetis yang akan ditulis ke AndroidManifest.xml.
#[derive(Debug, Clone)]
pub struct AppSpec {
    pub package: String,
    pub version_code: i32,
    pub version_name: String,
    pub min_sdk: i32,
    pub target_sdk: i32,
    pub label: Option<String>,
    pub debuggable: bool,
    pub permissions: Vec<String>,
    pub features: Vec<String>,
    pub activities: Vec<String>,
    /// Pasangan `<meta-data android:name android:value>` pada manifest.
    pub meta_data: Vec<(String, String)>,
}

impl Default for AppSpec {
    fn default() -> Self {
        Self {
            package: "com.example.fixture".to_string(),
            version_code: 1,
            version_name: "1.0".to_string(),
            min_sdk: 21,
            target_sdk: 34,
            label: Some("Fixture".to_string()),
            debuggable: false,
            permissions: Vec::new(),
            features: Vec::new(),
            activities: vec![".MainActivity".to_string()],
            meta_data: Vec::new(),
        }
    }
}

/// DEX minimal yang hanya berisi header valid tanpa data.
///
/// Cukup untuk menguji jalur inspeksi statis tanpa mengeksekusi apa pun.
pub fn minimal_dex() -> Vec<u8> {
    let mut dex = vec![0u8; 112];
    dex[..8].copy_from_slice(b"dex\n035\0");
    dex[32..36].copy_from_slice(&112u32.to_le_bytes());
    dex[36..40].copy_from_slice(&112u32.to_le_bytes());
    dex[40..44].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    dex
}

/// DEX minimal dengan jumlah string/type/method yang ditentukan pada header.
///
/// Offset sengaja nol sehingga parser tidak membaca melewati header; berguna
/// untuk menguji laporan info tanpa membangun tabel DEX lengkap.
pub fn header_only_dex(strings: u32, types: u32, methods: u32) -> Vec<u8> {
    let mut dex = minimal_dex();
    dex[56..60].copy_from_slice(&strings.to_le_bytes());
    dex[64..68].copy_from_slice(&types.to_le_bytes());
    dex[88..92].copy_from_slice(&methods.to_le_bytes());
    dex
}

#[derive(Debug, Clone)]
enum Value {
    Str(String),
    Int(i32),
    Bool(bool),
}

#[derive(Debug, Clone)]
struct Node {
    name: String,
    attrs: Vec<(String, Value)>,
    children: Vec<Node>,
}

impl Node {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            attrs: Vec::new(),
            children: Vec::new(),
        }
    }

    fn attr(mut self, name: &str, value: Value) -> Self {
        self.attrs.push((name.to_string(), value));
        self
    }

    fn child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }
}

fn manifest_tree(spec: &AppSpec) -> Node {
    let uses_sdk = Node::new("uses-sdk")
        .attr("minSdkVersion", Value::Int(spec.min_sdk))
        .attr("targetSdkVersion", Value::Int(spec.target_sdk));

    let mut application = Node::new("application").attr("debuggable", Value::Bool(spec.debuggable));
    if let Some(label) = &spec.label {
        application = application.attr("label", Value::Str(label.clone()));
    }
    for (name, value) in &spec.meta_data {
        application = application.child(
            Node::new("meta-data")
                .attr("name", Value::Str(name.clone()))
                .attr("value", Value::Str(value.clone())),
        );
    }
    for (index, activity) in spec.activities.iter().enumerate() {
        let mut node = Node::new("activity").attr("name", Value::Str(activity.clone()));
        // Activity pertama diperlakukan sebagai launcher, seperti aplikasi biasa.
        if index == 0 {
            node = node.child(
                Node::new("intent-filter")
                    .child(
                        Node::new("action")
                            .attr("name", Value::Str("android.intent.action.MAIN".to_string())),
                    )
                    .child(Node::new("category").attr(
                        "name",
                        Value::Str("android.intent.category.LAUNCHER".to_string()),
                    )),
            );
        }
        application = application.child(node);
    }

    let mut manifest = Node::new("manifest")
        .attr("package", Value::Str(spec.package.clone()))
        .attr("versionCode", Value::Int(spec.version_code))
        .attr("versionName", Value::Str(spec.version_name.clone()));

    for permission in &spec.permissions {
        manifest = manifest
            .child(Node::new("uses-permission").attr("name", Value::Str(permission.clone())));
    }
    for feature in &spec.features {
        manifest =
            manifest.child(Node::new("uses-feature").attr("name", Value::Str(feature.clone())));
    }

    manifest.child(uses_sdk).child(application)
}

#[derive(Default)]
struct StringPool {
    order: Vec<String>,
    index: HashMap<String, u32>,
}

impl StringPool {
    fn intern(&mut self, value: &str) -> u32 {
        if let Some(&index) = self.index.get(value) {
            return index;
        }
        let index = self.order.len() as u32;
        self.order.push(value.to_string());
        self.index.insert(value.to_string(), index);
        index
    }

    fn get(&self, value: &str) -> u32 {
        self.index
            .get(value)
            .copied()
            .expect("string harus sudah diintern sebelum serialisasi")
    }
}

fn collect_strings(node: &Node, pool: &mut StringPool) {
    pool.intern(&node.name);
    for (name, value) in &node.attrs {
        pool.intern(name);
        if let Value::Str(text) = value {
            pool.intern(text);
        }
    }
    for child in &node.children {
        collect_strings(child, pool);
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn chunk(kind: u16, header_size: u16, body: &[u8]) -> Vec<u8> {
    // `body` adalah seluruh byte setelah header chunk (8 byte), sehingga ukuran
    // total chunk selalu 8 + panjang body, bukan header_size + panjang body.
    assert!(
        usize::from(header_size) <= 8 + body.len(),
        "header_size melebihi ukuran chunk"
    );
    let size = u32::try_from(8 + body.len()).expect("chunk terlalu besar");
    let mut out = Vec::with_capacity(size as usize);
    push_u16(&mut out, kind);
    push_u16(&mut out, header_size);
    push_u32(&mut out, size);
    out.extend_from_slice(body);
    out
}

fn utf8_length(out: &mut Vec<u8>, len: usize) {
    assert!(len < 0x80, "panjang string fixture harus < 128");
    out.push(len as u8);
}

fn encode_string_pool(pool: &StringPool) -> Vec<u8> {
    let mut data = Vec::new();
    let mut offsets = Vec::with_capacity(pool.order.len());

    for value in &pool.order {
        offsets.push(u32::try_from(data.len()).expect("string pool terlalu besar"));
        let bytes = value.as_bytes();
        utf8_length(&mut data, value.chars().count());
        utf8_length(&mut data, bytes.len());
        data.extend_from_slice(bytes);
        data.push(0);
    }
    while data.len() % 4 != 0 {
        data.push(0);
    }

    let count = u32::try_from(pool.order.len()).expect("terlalu banyak string");
    let strings_start = 28 + count * 4;

    let mut body = Vec::new();
    push_u32(&mut body, count);
    push_u32(&mut body, 0);
    push_u32(&mut body, UTF8_FLAG);
    push_u32(&mut body, strings_start);
    push_u32(&mut body, 0);
    for offset in offsets {
        push_u32(&mut body, offset);
    }
    body.extend_from_slice(&data);

    chunk(CHUNK_STRING_POOL, 28, &body)
}

fn encode_start_element(node: &Node, pool: &StringPool) -> Vec<u8> {
    let count = u16::try_from(node.attrs.len()).expect("terlalu banyak atribut");

    let mut body = Vec::new();
    push_u32(&mut body, 1);
    push_u32(&mut body, NO_INDEX);
    push_u32(&mut body, NO_INDEX);
    push_u32(&mut body, pool.get(&node.name));
    push_u16(&mut body, 0x14);
    push_u16(&mut body, 0x14);
    push_u16(&mut body, count);
    push_u16(&mut body, 0);
    push_u16(&mut body, 0);
    push_u16(&mut body, 0);

    for (name, value) in &node.attrs {
        push_u32(&mut body, NO_INDEX);
        push_u32(&mut body, pool.get(name));

        let (raw, kind, data) = match value {
            Value::Str(text) => {
                let index = pool.get(text);
                (index, TYPE_STRING, index)
            }
            Value::Int(number) => (NO_INDEX, TYPE_INT_DEC, *number as u32),
            Value::Bool(flag) => (NO_INDEX, TYPE_INT_BOOLEAN, u32::from(*flag)),
        };

        push_u32(&mut body, raw);
        push_u16(&mut body, 8);
        body.push(0);
        body.push(kind);
        push_u32(&mut body, data);
    }

    chunk(CHUNK_START_ELEMENT, 16, &body)
}

fn encode_end_element(node: &Node, pool: &StringPool) -> Vec<u8> {
    let mut body = Vec::new();
    push_u32(&mut body, 1);
    push_u32(&mut body, NO_INDEX);
    push_u32(&mut body, NO_INDEX);
    push_u32(&mut body, pool.get(&node.name));
    chunk(CHUNK_END_ELEMENT, 16, &body)
}

fn encode_namespace(kind: u16, pool: &StringPool) -> Vec<u8> {
    let mut body = Vec::new();
    push_u32(&mut body, 1);
    push_u32(&mut body, NO_INDEX);
    push_u32(&mut body, pool.get(ANDROID_PREFIX));
    push_u32(&mut body, pool.get(ANDROID_NS));
    chunk(kind, 16, &body)
}

fn emit_tree(node: &Node, pool: &StringPool, out: &mut Vec<u8>) {
    out.extend_from_slice(&encode_start_element(node, pool));
    for child in &node.children {
        emit_tree(child, pool, out);
    }
    out.extend_from_slice(&encode_end_element(node, pool));
}

/// Bangun AndroidManifest.xml dalam format binary XML (AXML) yang valid.
///
/// Atribut sengaja ditulis tanpa resource map sehingga nama atribut dibaca
/// langsung dari string pool.
pub fn manifest_axml(spec: &AppSpec) -> Vec<u8> {
    let root = manifest_tree(spec);

    let mut pool = StringPool::default();
    pool.intern(ANDROID_PREFIX);
    pool.intern(ANDROID_NS);
    collect_strings(&root, &mut pool);

    let pool_chunk = encode_string_pool(&pool);
    let map_chunk = chunk(CHUNK_RESOURCE_MAP, 8, &[]);
    let start_ns = encode_namespace(CHUNK_START_NAMESPACE, &pool);
    let end_ns = encode_namespace(CHUNK_END_NAMESPACE, &pool);

    let mut body = Vec::new();
    body.extend_from_slice(&pool_chunk);
    body.extend_from_slice(&map_chunk);
    body.extend_from_slice(&start_ns);
    emit_tree(&root, &pool, &mut body);
    body.extend_from_slice(&end_ns);

    chunk(CHUNK_XML, 8, &body)
}

fn zip_entry(
    zip: &mut ZipWriter<Cursor<Vec<u8>>>,
    name: &str,
    bytes: &[u8],
    compression: CompressionMethod,
) {
    let options = SimpleFileOptions::default().compression_method(compression);
    zip.start_file(name, options)
        .expect("tidak dapat memulai entry zip fixture");
    zip.write_all(bytes)
        .expect("tidak dapat menulis entry zip fixture");
}

/// Bangun APK sintetis dengan daftar entry tambahan (mis. classes2.dex).
pub fn apk_bytes_with_entries(spec: &AppSpec, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip_entry(
        &mut zip,
        "AndroidManifest.xml",
        &manifest_axml(spec),
        CompressionMethod::Deflated,
    );
    for (name, bytes) in entries {
        zip_entry(&mut zip, name, bytes, CompressionMethod::Stored);
    }
    zip.finish()
        .expect("tidak dapat menyelesaikan zip fixture")
        .into_inner()
}

/// Bangun APK sintetis di memori: manifest AXML + classes.dex.
pub fn apk_bytes_with_dex(spec: &AppSpec, dex: &[u8]) -> Vec<u8> {
    apk_bytes_with_entries(spec, &[("classes.dex", dex)])
}

/// Bangun APK sintetis dengan classes.dex dan classes2.dex.
pub fn multidex_apk_bytes(spec: &AppSpec) -> Vec<u8> {
    let first = minimal_dex();
    let second = header_only_dex(1, 0, 0);
    apk_bytes_with_entries(spec, &[("classes.dex", &first), ("classes2.dex", &second)])
}

/// Bangun APK sintetis di memori dengan DEX minimal.
pub fn apk_bytes(spec: &AppSpec) -> Vec<u8> {
    apk_bytes_with_dex(spec, &minimal_dex())
}

/// Tulis APK sintetis ke path tujuan.
pub fn write_apk(path: &Path, spec: &AppSpec) -> io::Result<()> {
    std::fs::write(path, apk_bytes(spec))
}

/// Bangun container XAPK: manifest.json + <package>.apk di dalam ZIP.
pub fn xapk_bytes(spec: &AppSpec) -> Vec<u8> {
    let inner = apk_bytes(spec);
    let manifest = format!("{{\"package_name\":\"{}\"}}", spec.package);

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip_entry(
        &mut zip,
        "manifest.json",
        manifest.as_bytes(),
        CompressionMethod::Deflated,
    );
    let inner_name = format!("{}.apk", spec.package);
    zip_entry(&mut zip, &inner_name, &inner, CompressionMethod::Stored);
    zip.finish()
        .expect("tidak dapat menyelesaikan xapk fixture")
        .into_inner()
}

/// Bangun container APKM: base.apk di dalam ZIP.
pub fn apkm_bytes(spec: &AppSpec) -> Vec<u8> {
    let inner = apk_bytes(spec);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip_entry(&mut zip, "base.apk", &inner, CompressionMethod::Stored);
    zip.finish()
        .expect("tidak dapat menyelesaikan apkm fixture")
        .into_inner()
}

/// Bangun ZIP generik tanpa AndroidManifest.xml, untuk menguji penolakan input.
pub fn plain_zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip_entry(&mut zip, name, bytes, CompressionMethod::Deflated);
    }
    zip.finish()
        .expect("tidak dapat menyelesaikan zip fixture")
        .into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_axml_memiliki_header_xml_dan_string_pool() {
        let axml = manifest_axml(&AppSpec::default());
        assert!(axml.len() > 8);
        assert_eq!(u16::from_le_bytes([axml[0], axml[1]]), CHUNK_XML);
        assert_eq!(
            u32::from_le_bytes([axml[4], axml[5], axml[6], axml[7]]) as usize,
            axml.len()
        );
        assert_eq!(u16::from_le_bytes([axml[8], axml[9]]), CHUNK_STRING_POOL);
    }

    #[test]
    fn minimal_dex_punya_magic_dan_ukuran_header() {
        let dex = minimal_dex();
        assert_eq!(&dex[..8], b"dex\n035\0");
        assert_eq!(
            u32::from_le_bytes([dex[32], dex[33], dex[34], dex[35]]),
            112
        );
        assert_eq!(
            u32::from_le_bytes([dex[36], dex[37], dex[38], dex[39]]),
            112
        );
    }

    #[test]
    fn apk_bytes_berisi_manifest_dan_dex() {
        let bytes = apk_bytes(&AppSpec::default());
        let archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip fixture harus valid");
        let names: Vec<&str> = archive.file_names().collect();
        assert!(names.contains(&"AndroidManifest.xml"));
        assert!(names.contains(&"classes.dex"));
    }

    #[test]
    fn xapk_dan_apkm_memakai_nama_entry_yang_diharapkan() {
        let spec = AppSpec::default();
        let xapk = zip::ZipArchive::new(Cursor::new(xapk_bytes(&spec))).expect("xapk valid");
        assert!(xapk.file_names().any(|name| name == "manifest.json"));

        let apkm = zip::ZipArchive::new(Cursor::new(apkm_bytes(&spec))).expect("apkm valid");
        assert!(apkm.file_names().any(|name| name == "base.apk"));
    }
}
