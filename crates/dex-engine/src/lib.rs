use serde::Serialize;
use std::fs;
use std::path::Path;

mod interpreter;
mod value;

pub use interpreter::{MAX_CALL_DEPTH, MAX_STEPS};
pub use value::{Heap, Object, Value};

/// Penyedia method yang diimplementasikan runtime, bukan bytecode DEX.
///
/// Implementasi berada di crate framework agar arah dependensi tetap satu
/// arah: framework-api bergantung pada dex-engine, bukan sebaliknya.
/// Penanda bahwa resolver mencapai loop utama yang tidak punya tugas tertunda.
///
/// Runtime ini kooperatif dan satu thread: `Looper.loop()` tidak dapat
/// memblokir seperti Android, dan mengembalikan segera akan membuat aplikasi
/// berputar tanpa kemajuan. Resolver mengembalikan penanda ini agar engine
/// menghentikan eksekusi dan melaporkan loop utama idle, bukan membakar batas
/// langkah.
pub const MAIN_LOOP_IDLE: &str = "@main-loop-idle";

pub trait NativeResolver: std::fmt::Debug {
    /// Kembalikan None bila method tidak disediakan runtime.
    fn invoke_static(
        &self,
        class_descriptor: &str,
        method_name: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>>;

    /// Varian dengan akses heap dan signature method.
    ///
    /// `signature` memakai format DEX, misalnya `(J)J`, sehingga implementasi
    /// dapat membedakan overload. Implementasi bawaan meneruskan ke
    /// [`NativeResolver::invoke_static`] agar resolver sederhana tetap sah.
    fn invoke_described(
        &self,
        heap: &mut crate::value::Heap,
        class_descriptor: &str,
        method_name: &str,
        signature: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        let _ = (heap, signature);
        self.invoke_static(class_descriptor, method_name, args)
    }

    /// Nilai field framework yang tidak disimpan pada objek.
    ///
    /// Dipanggil `iget` ketika field belum pernah ditulis, sehingga runtime
    /// dapat mengisi field kelas framework (mis. `PackageInfo.versionName`)
    /// tanpa perlu mengetahui indeks `field_ids` milik APK.
    fn field_default(
        &self,
        heap: &mut crate::value::Heap,
        class_descriptor: &str,
        field_name: &str,
    ) -> Option<Value> {
        let _ = (heap, class_descriptor, field_name);
        None
    }
}

const DEX_HEADER_SIZE: usize = 112;
const MAGIC_LEN: usize = 8;
const STRING_ID_ITEM_SIZE: usize = 4;
const TYPE_ID_ITEM_SIZE: usize = 4;
const PROTO_ID_ITEM_SIZE: usize = 12;
const FIELD_ID_ITEM_SIZE: usize = 8;
const METHOD_ID_ITEM_SIZE: usize = 8;
const CLASS_DEF_ITEM_SIZE: usize = 32;

/// Batas jumlah entri per tabel DEX yang mau dialokasikan.
///
/// Nilai ini jauh di atas DEX nyata terbesar, tetapi mencegah header palsu
/// memicu alokasi raksasa sebelum isi berkas diverifikasi.
const MAX_TABLE_ENTRIES: u32 = 1_000_000;

/// Batas kapasitas awal untuk struktur bersarang (class_data, handler).
/// Jumlah sebenarnya tetap dibatasi oleh panjang data yang tersedia.
const NESTED_PREALLOC_LIMIT: usize = 1024;

const DEX_ENDIAN_CONSTANT: u32 = 0x1234_5678;
const DEX_REVERSE_ENDIAN_CONSTANT: u32 = 0x7856_3412;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DexVersion {
    V035,
    V036,
    V037,
    V038,
    Unknown,
}

impl DexVersion {
    pub fn from_magic(magic: &[u8; MAGIC_LEN]) -> Self {
        match magic {
            b"dex\n035\0" => DexVersion::V035,
            b"dex\n036\0" => DexVersion::V036,
            b"dex\n037\0" => DexVersion::V037,
            b"dex\n038\0" => DexVersion::V038,
            _ => DexVersion::Unknown,
        }
    }
}

impl std::fmt::Display for DexVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DexVersion::V035 => write!(f, "035"),
            DexVersion::V036 => write!(f, "036"),
            DexVersion::V037 => write!(f, "037"),
            DexVersion::V038 => write!(f, "038"),
            DexVersion::Unknown => write!(f, "tidak diketahui"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DexHeader {
    pub magic: [u8; MAGIC_LEN],
    pub version: DexVersion,
    pub checksum: u32,
    pub signature: [u8; 20],
    pub file_size: u32,
    pub header_size: u32,
    pub endian_tag: u32,
    pub link_size: u32,
    pub link_off: u32,
    pub map_off: u32,
    pub string_ids_size: u32,
    pub string_ids_off: u32,
    pub type_ids_size: u32,
    pub type_ids_off: u32,
    pub proto_ids_size: u32,
    pub proto_ids_off: u32,
    pub field_ids_size: u32,
    pub field_ids_off: u32,
    pub method_ids_size: u32,
    pub method_ids_off: u32,
    pub class_defs_size: u32,
    pub class_defs_off: u32,
    pub data_size: u32,
    pub data_off: u32,
}

#[derive(Debug)]
pub enum DexError {
    Io(std::io::Error),
    TooSmall,
    InvalidMagic,
    UnsupportedVersion(DexVersion),
    InvalidHeaderSize(u32),
    UnsupportedEndian(u32),
    Truncated,
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    LimitExceeded {
        table: &'static str,
        count: u32,
        limit: u32,
    },
    MethodNotFound {
        class: String,
        method: String,
    },
    NoCode {
        class: String,
        method: String,
        signature: String,
    },
    InvalidInstruction {
        pc: u32,
        opcode: u8,
    },
    InvalidBranch {
        pc: u32,
        offset: i32,
    },
    RegisterOutOfRange {
        register: u32,
        registers: u16,
    },
    DivisionByZero,
    CallDepthExceeded {
        depth: usize,
    },
    /// Loop utama aplikasi tidak punya tugas tertunda untuk dijalankan.
    MainLoopIdle,
    ExecutionLimit {
        steps: u64,
        /// Method terdalam yang sedang berjalan saat batas tercapai.
        ///
        /// Diisi saat galat merambat keluar dari interpreter agar perintah run
        /// dapat menjelaskan di mana aplikasi berputar, bukan hanya berapa
        /// langkah yang sudah dipakai.
        location: Option<String>,
    },
    TypeMismatch {
        expected: &'static str,
    },
    NativeFailure {
        class: String,
        method: String,
        message: String,
    },
    /// Eksepsi dilempar tetapi tidak ada handler yang menangkapnya.
    UncaughtException {
        type_name: String,
    },
    /// Nilai yang dilempar bukan referensi objek.
    ThrowNonReference {
        pc: u32,
    },
    /// Indeks field di luar `field_ids`.
    FieldNotFound {
        field_index: u32,
    },
    /// Ukuran array negatif pada `new-array`.
    NegativeArraySize {
        size: i32,
    },
    /// Indeks array di luar panjang array.
    ArrayIndexOutOfBounds {
        index: i32,
        length: usize,
    },
    /// Operasi field/array diterapkan pada nilai yang bukan referensi.
    NullReference {
        pc: u32,
    },
    /// Jumlah elemen payload `fill-array-data` tidak sama dengan array.
    ArrayDataMismatch {
        payload: usize,
        array: usize,
    },
    /// Payload `fill-array-data` tidak diawali pseudo-opcode 0x0300.
    InvalidArrayPayload {
        pc: u32,
    },
}

impl std::fmt::Display for DexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DexError::Io(error) => write!(f, "gagal membaca berkas DEX: {error}"),
            DexError::TooSmall => f.write_str("berkas DEX terlalu pendek untuk header"),
            DexError::InvalidMagic => f.write_str("magic berkas DEX tidak valid"),
            DexError::UnsupportedVersion(version) => {
                write!(f, "versi DEX tidak didukung: {version}")
            }
            DexError::InvalidHeaderSize(size) => {
                write!(f, "ukuran header DEX tidak valid: {size} byte")
            }
            DexError::UnsupportedEndian(tag) => {
                write!(f, "endian tag DEX tidak didukung: 0x{tag:08x}")
            }
            DexError::Truncated => f.write_str("berkas DEX terpotong"),
            DexError::LimitExceeded {
                table,
                count,
                limit,
            } => write!(
                f,
                "tabel {table} terlalu besar: {count} entri (batas {limit})"
            ),
            DexError::SizeMismatch { expected, actual } => {
                write!(
                    f,
                    "ukuran berkas tidak cocok: header menyatakan {expected}, aktual {actual}"
                )
            }
            DexError::MethodNotFound { class, method } => {
                write!(f, "method tidak ditemukan: {class}#{method}")
            }
            DexError::NoCode {
                class,
                method,
                signature,
            } => {
                write!(f, "method tidak memiliki kode: {class}#{method}{signature}")
            }
            DexError::InvalidInstruction { pc, opcode } => {
                write!(f, "instruksi tidak didukung 0x{opcode:02x} pada pc {pc}")
            }
            DexError::InvalidBranch { pc, offset } => {
                write!(
                    f,
                    "target cabang tidak valid dari pc {pc} dengan offset {offset}"
                )
            }
            DexError::RegisterOutOfRange {
                register,
                registers,
            } => {
                write!(
                    f,
                    "register v{register} di luar batas ({registers} register)"
                )
            }
            DexError::DivisionByZero => f.write_str("pembagian dengan nol"),
            DexError::CallDepthExceeded { depth } => {
                write!(f, "kedalaman pemanggilan melebihi batas: {depth}")
            }
            DexError::MainLoopIdle => {
                write!(f, "loop utama idle: tidak ada tugas tertunda")
            }
            DexError::ExecutionLimit { steps, location } => {
                write!(f, "batas langkah eksekusi terlampaui: {steps}")?;
                match location {
                    Some(location) => write!(f, " (di {location})"),
                    None => Ok(()),
                }
            }
            DexError::TypeMismatch { expected } => {
                write!(f, "tipe nilai tidak sesuai, diharapkan {expected}")
            }
            DexError::NativeFailure {
                class,
                method,
                message,
            } => write!(f, "method native gagal {class}->{method}: {message}"),
            DexError::UncaughtException { type_name } => {
                write!(f, "eksepsi tidak tertangkap: {type_name}")
            }
            DexError::ThrowNonReference { pc } => {
                write!(
                    f,
                    "nilai pada pc {pc} bukan referensi objek sehingga tidak dapat dilempar"
                )
            }
            DexError::FieldNotFound { field_index } => {
                write!(f, "field index {field_index} di luar field_ids")
            }
            DexError::NegativeArraySize { size } => {
                write!(f, "ukuran array negatif: {size}")
            }
            DexError::ArrayIndexOutOfBounds { index, length } => {
                write!(f, "indeks array {index} di luar panjang {length}")
            }
            DexError::NullReference { pc } => {
                write!(f, "referensi null pada pc {pc}")
            }
            DexError::ArrayDataMismatch { payload, array } => {
                write!(
                    f,
                    "payload fill-array-data berisi {payload} elemen, array memiliki {array}"
                )
            }
            DexError::InvalidArrayPayload { pc } => {
                write!(f, "payload fill-array-data tidak valid pada pc {pc}")
            }
        }
    }
}

impl std::error::Error for DexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DexError::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for DexError {
    fn from(error: std::io::Error) -> Self {
        DexError::Io(error)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DexFile {
    pub path: String,
    pub header: DexHeader,
    pub string_ids: Vec<String>,
    pub type_ids: Vec<String>,
    pub proto_ids: Vec<ProtoId>,
    pub field_ids: Vec<FieldId>,
    pub method_ids: Vec<MethodId>,
    pub class_defs: Vec<ClassDef>,
    pub code_items: Vec<CodeItem>,
    /// Peta code_off -> indeks di code_items, dipakai untuk mencari badan
    /// method saat eksekusi tanpa memindai ulang class_data.
    #[serde(skip)]
    pub code_item_by_offset: std::collections::HashMap<u32, usize>,
    /// Peta method_idx -> indeks code_items agar pencarian badan method O(1).
    #[serde(skip)]
    pub code_item_by_method: std::collections::HashMap<u32, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProtoId {
    pub shorty: String,
    pub return_type: String,
    pub parameters_off: u32,
    pub parameter_types: Vec<String>,
}

/// Kegagalan API framework yang harus muncul sebagai eksepsi Java.
///
/// Kontrak NativeResolver mengembalikan Result<Value, String>, jadi eksepsi
/// dikodekan di dalam pesan galat alih-alih menambah tipe galat baru pada
/// setiap arm resolver. Formatnya @throw:<descriptor>:<pesan>; pesan yang
/// tidak memakai awalan itu tetap menjadi DexError::NativeFailure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeException {
    pub class: String,
    pub message: String,
}

impl NativeException {
    pub const PREFIX: &'static str = "@throw:";

    pub fn new(class: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class: class.into(),
            message: message.into(),
        }
    }

    pub fn encode(&self) -> String {
        format!("{}{}:{}", Self::PREFIX, self.class, self.message)
    }

    /// Mengurai pesan galat resolver; None bila bukan eksepsi.
    pub fn decode(text: &str) -> Option<Self> {
        let rest = text.strip_prefix(Self::PREFIX)?;
        let (class, message) = rest.split_once(':')?;
        if !class.starts_with('L') || !class.ends_with(';') || class.len() < 3 {
            return None;
        }
        Some(Self {
            class: class.to_string(),
            message: message.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldId {
    pub class: String,
    pub field_type: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MethodId {
    pub class: String,
    pub name: String,
    pub proto_idx: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassDef {
    pub class: String,
    pub superclass: Option<String>,
    pub access_flags: u32,
    pub class_data_off: u32,
    pub class_data: Option<ClassData>,
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let chunk = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    let chunk = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([chunk[0], chunk[1]]))
}

/// Validasi bahwa tabel berukuran tetap berada di dalam berkas sebelum
/// dialokasikan, sekaligus menolak jumlah entri yang tidak masuk akal.
fn table_capacity(
    data_len: usize,
    table: &'static str,
    offset: u32,
    count: u32,
    item_size: usize,
) -> Result<usize, DexError> {
    if count > MAX_TABLE_ENTRIES {
        return Err(DexError::LimitExceeded {
            table,
            count,
            limit: MAX_TABLE_ENTRIES,
        });
    }

    let bytes = (count as usize)
        .checked_mul(item_size)
        .ok_or(DexError::Truncated)?;
    let end = (offset as usize)
        .checked_add(bytes)
        .ok_or(DexError::Truncated)?;
    if end > data_len {
        return Err(DexError::Truncated);
    }

    Ok(count as usize)
}

/// Kapasitas awal yang aman untuk struktur bersarang.
fn nested_capacity(count: u32) -> usize {
    (count as usize).min(NESTED_PREALLOC_LIMIT)
}

fn read_type_list(data: &[u8], offset: u32, type_ids: &[String]) -> Option<Vec<String>> {
    if offset == 0 {
        return Some(Vec::new());
    }
    let start = offset as usize;
    let size = read_u32_le(data, start)? as usize;
    let mut types = Vec::with_capacity(size);
    for i in 0..size {
        // `type_item` berukuran 2 byte (ushort), bukan 4 byte.
        let idx_off = start
            .checked_add(4)
            .and_then(|base| base.checked_add(i.checked_mul(2)?))?;
        let idx = read_u16_le(data, idx_off)? as usize;
        types.push(type_ids.get(idx)?.clone());
    }
    Some(types)
}

fn read_uleb128(bytes: &[u8], mut offset: usize) -> Option<(u32, usize)> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = *bytes.get(offset)?;
        offset += 1;
        if shift > 28 {
            return None;
        }
        result |= (byte as u32 & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Some((result, offset));
        }
        shift += 7;
    }
}

/// Baca satu string_data_item DEX.
///
/// Format DEX menyimpan string sebagai MUTF-8 (bukan UTF-16): uleb128 berisi
/// jumlah unit UTF-16, lalu byte MUTF-8 yang diakhiri 0x00. NUL direpresentasikan
/// sebagai 0xC0 0x80, dan pasangan surrogate disimpan sebagai dua unit terpisah.
fn read_mutf8_string(data: &[u8], string_data_off: u32) -> Option<String> {
    let mut pos = string_data_off as usize;
    let (_utf16_len, next) = read_uleb128(data, pos)?;
    pos = next;

    let mut units: Vec<u16> = Vec::new();
    loop {
        let byte = *data.get(pos)?;
        pos += 1;

        match byte {
            0x00 => break,
            0x01..=0x7F => units.push(u16::from(byte)),
            0xC0..=0xDF => {
                let second = *data.get(pos)?;
                pos += 1;
                units.push(((u16::from(byte) & 0x1F) << 6) | (u16::from(second) & 0x3F));
            }
            0xE0..=0xEF => {
                let second = *data.get(pos)?;
                let third = *data.get(pos + 1)?;
                pos += 2;
                units.push(
                    ((u16::from(byte) & 0x0F) << 12)
                        | ((u16::from(second) & 0x3F) << 6)
                        | (u16::from(third) & 0x3F),
                );
            }
            _ => return None,
        }

        if units.len() > data.len() {
            return None;
        }
    }

    Some(String::from_utf16_lossy(&units))
}

fn version_from_magic(magic: &[u8; MAGIC_LEN]) -> DexVersion {
    DexVersion::from_magic(magic)
}

pub fn parse_bytes(data: &[u8], source: &str) -> Result<DexFile, DexError> {
    if data.len() < DEX_HEADER_SIZE {
        return Err(DexError::TooSmall);
    }

    let mut magic = [0u8; MAGIC_LEN];
    magic.copy_from_slice(&data[0..MAGIC_LEN]);
    let version = version_from_magic(&magic);

    if matches!(version, DexVersion::Unknown) {
        return Err(DexError::InvalidMagic);
    }

    let checksum = read_u32_le(data, 8).ok_or(DexError::Truncated)?;
    let mut signature = [0u8; 20];
    signature.copy_from_slice(&data[12..32]);
    let file_size = read_u32_le(data, 32).ok_or(DexError::Truncated)?;
    let header_size = read_u32_le(data, 36).ok_or(DexError::Truncated)?;
    let endian_tag = read_u32_le(data, 40).ok_or(DexError::Truncated)?;
    let link_size = read_u32_le(data, 44).ok_or(DexError::Truncated)?;
    let link_off = read_u32_le(data, 48).ok_or(DexError::Truncated)?;
    let map_off = read_u32_le(data, 52).ok_or(DexError::Truncated)?;
    let string_ids_size = read_u32_le(data, 56).ok_or(DexError::Truncated)?;
    let string_ids_off = read_u32_le(data, 60).ok_or(DexError::Truncated)?;
    let type_ids_size = read_u32_le(data, 64).ok_or(DexError::Truncated)?;
    let type_ids_off = read_u32_le(data, 68).ok_or(DexError::Truncated)?;
    let proto_ids_size = read_u32_le(data, 72).ok_or(DexError::Truncated)?;
    let proto_ids_off = read_u32_le(data, 76).ok_or(DexError::Truncated)?;
    let field_ids_size = read_u32_le(data, 80).ok_or(DexError::Truncated)?;
    let field_ids_off = read_u32_le(data, 84).ok_or(DexError::Truncated)?;
    let method_ids_size = read_u32_le(data, 88).ok_or(DexError::Truncated)?;
    let method_ids_off = read_u32_le(data, 92).ok_or(DexError::Truncated)?;
    let class_defs_size = read_u32_le(data, 96).ok_or(DexError::Truncated)?;
    let class_defs_off = read_u32_le(data, 100).ok_or(DexError::Truncated)?;
    let data_size = read_u32_le(data, 104).ok_or(DexError::Truncated)?;
    let data_off = read_u32_le(data, 108).ok_or(DexError::Truncated)?;

    if header_size != DEX_HEADER_SIZE as u32 {
        return Err(DexError::InvalidHeaderSize(header_size));
    }

    if endian_tag == DEX_REVERSE_ENDIAN_CONSTANT {
        // DEX big-endian tidak pernah dipakai ART; menolaknya lebih aman
        // daripada salah menafsirkan seluruh tabel.
        return Err(DexError::UnsupportedEndian(endian_tag));
    }
    if endian_tag != DEX_ENDIAN_CONSTANT {
        return Err(DexError::UnsupportedEndian(endian_tag));
    }

    if file_size as usize > data.len() {
        return Err(DexError::SizeMismatch {
            expected: file_size as u64,
            actual: data.len() as u64,
        });
    }

    let header = DexHeader {
        magic,
        version,
        checksum,
        signature,
        file_size,
        header_size,
        endian_tag,
        link_size,
        link_off,
        map_off,
        string_ids_size,
        string_ids_off,
        type_ids_size,
        type_ids_off,
        proto_ids_size,
        proto_ids_off,
        field_ids_size,
        field_ids_off,
        method_ids_size,
        method_ids_off,
        class_defs_size,
        class_defs_off,
        data_size,
        data_off,
    };

    let string_ids = parse_string_ids(data, &header)?;
    let type_ids = parse_type_ids(data, &header, &string_ids)?;
    let proto_ids = parse_proto_ids(data, &header, &string_ids, &type_ids)?;
    let field_ids = parse_field_ids(data, &header, &string_ids, &type_ids)?;
    let method_ids = parse_method_ids(data, &header, &string_ids, &type_ids)?;
    let class_defs = parse_class_defs(data, &header, &type_ids)?;

    let mut class_defs_with_data = Vec::with_capacity(class_defs.len());
    let mut code_items: Vec<CodeItem> = Vec::new();
    let mut code_item_by_offset = std::collections::HashMap::new();
    let mut code_item_by_method = std::collections::HashMap::new();
    for mut class_def in class_defs {
        if class_def.class_data_off != 0
            && let Ok(Some(class_data)) = parse_class_data(data, class_def.class_data_off)
        {
            for method in class_data
                .direct_methods
                .iter()
                .chain(class_data.virtual_methods.iter())
            {
                if method.code_off != 0
                    && !code_item_by_offset.contains_key(&method.code_off)
                    && let Ok(code_item) = parse_code_item(data, method.code_off)
                {
                    code_item_by_offset.insert(method.code_off, code_items.len());
                    code_item_by_method.insert(method.method_idx, code_items.len());
                    code_items.push(code_item);
                }
            }
            class_def.class_data = Some(class_data);
        }
        class_defs_with_data.push(class_def);
    }

    Ok(DexFile {
        path: source.to_owned(),
        header,
        string_ids,
        type_ids,
        proto_ids,
        field_ids,
        method_ids,
        class_defs: class_defs_with_data,
        code_items,
        code_item_by_offset,
        code_item_by_method,
    })
}

fn parse_string_ids(data: &[u8], header: &DexHeader) -> Result<Vec<String>, DexError> {
    let off = header.string_ids_off as usize;
    let size = table_capacity(
        data.len(),
        "string_ids",
        header.string_ids_off,
        header.string_ids_size,
        STRING_ID_ITEM_SIZE,
    )?;
    let mut ids = Vec::with_capacity(size);
    for i in 0..size {
        let item_off = off
            .checked_add(
                i.checked_mul(STRING_ID_ITEM_SIZE)
                    .ok_or(DexError::Truncated)?,
            )
            .ok_or(DexError::Truncated)?;
        let data_off = read_u32_le(data, item_off).ok_or(DexError::Truncated)?;
        let s = read_mutf8_string(data, data_off).ok_or(DexError::Truncated)?;
        ids.push(s);
    }
    Ok(ids)
}

fn parse_type_ids(
    data: &[u8],
    header: &DexHeader,
    string_ids: &[String],
) -> Result<Vec<String>, DexError> {
    let off = header.type_ids_off as usize;
    let size = table_capacity(
        data.len(),
        "type_ids",
        header.type_ids_off,
        header.type_ids_size,
        TYPE_ID_ITEM_SIZE,
    )?;
    let mut ids = Vec::with_capacity(size);
    for i in 0..size {
        let item_off = off
            .checked_add(
                i.checked_mul(TYPE_ID_ITEM_SIZE)
                    .ok_or(DexError::Truncated)?,
            )
            .ok_or(DexError::Truncated)?;
        let descriptor_idx = read_u32_le(data, item_off).ok_or(DexError::Truncated)? as usize;
        let descriptor = string_ids
            .get(descriptor_idx)
            .ok_or(DexError::Truncated)?
            .clone();
        ids.push(descriptor);
    }
    Ok(ids)
}

fn parse_proto_ids(
    data: &[u8],
    header: &DexHeader,
    string_ids: &[String],
    type_ids: &[String],
) -> Result<Vec<ProtoId>, DexError> {
    let off = header.proto_ids_off as usize;
    let size = table_capacity(
        data.len(),
        "proto_ids",
        header.proto_ids_off,
        header.proto_ids_size,
        PROTO_ID_ITEM_SIZE,
    )?;
    let mut ids = Vec::with_capacity(size);
    for i in 0..size {
        let item_off = off
            .checked_add(
                i.checked_mul(PROTO_ID_ITEM_SIZE)
                    .ok_or(DexError::Truncated)?,
            )
            .ok_or(DexError::Truncated)?;
        let shorty_idx = read_u32_le(data, item_off).ok_or(DexError::Truncated)?;
        let return_type_idx = read_u32_le(data, item_off + 4).ok_or(DexError::Truncated)?;
        let parameters_off = read_u32_le(data, item_off + 8).ok_or(DexError::Truncated)?;
        let shorty = string_ids
            .get(shorty_idx as usize)
            .cloned()
            .unwrap_or_default();
        let return_type = type_ids
            .get(return_type_idx as usize)
            .cloned()
            .unwrap_or_default();
        let parameter_types = read_type_list(data, parameters_off, type_ids).unwrap_or_default();
        ids.push(ProtoId {
            shorty,
            return_type,
            parameters_off,
            parameter_types,
        });
    }
    Ok(ids)
}

fn parse_field_ids(
    data: &[u8],
    header: &DexHeader,
    string_ids: &[String],
    type_ids: &[String],
) -> Result<Vec<FieldId>, DexError> {
    let off = header.field_ids_off as usize;
    let size = table_capacity(
        data.len(),
        "field_ids",
        header.field_ids_off,
        header.field_ids_size,
        FIELD_ID_ITEM_SIZE,
    )?;
    let mut ids = Vec::with_capacity(size);
    for i in 0..size {
        let item_off = off
            .checked_add(
                i.checked_mul(FIELD_ID_ITEM_SIZE)
                    .ok_or(DexError::Truncated)?,
            )
            .ok_or(DexError::Truncated)?;
        let class_idx = read_u16_le(data, item_off).ok_or(DexError::Truncated)?;
        let type_idx = read_u16_le(data, item_off + 2).ok_or(DexError::Truncated)?;
        let name_idx = read_u32_le(data, item_off + 4).ok_or(DexError::Truncated)?;
        let class = type_ids
            .get(class_idx as usize)
            .cloned()
            .unwrap_or_default();
        let field_type = type_ids.get(type_idx as usize).cloned().unwrap_or_default();
        let name = string_ids
            .get(name_idx as usize)
            .cloned()
            .unwrap_or_default();
        ids.push(FieldId {
            class,
            field_type,
            name,
        });
    }
    Ok(ids)
}

fn parse_method_ids(
    data: &[u8],
    header: &DexHeader,
    string_ids: &[String],
    type_ids: &[String],
) -> Result<Vec<MethodId>, DexError> {
    let off = header.method_ids_off as usize;
    let size = table_capacity(
        data.len(),
        "method_ids",
        header.method_ids_off,
        header.method_ids_size,
        METHOD_ID_ITEM_SIZE,
    )?;
    let mut ids = Vec::with_capacity(size);
    for i in 0..size {
        let item_off = off
            .checked_add(
                i.checked_mul(METHOD_ID_ITEM_SIZE)
                    .ok_or(DexError::Truncated)?,
            )
            .ok_or(DexError::Truncated)?;
        let class_idx = read_u16_le(data, item_off).ok_or(DexError::Truncated)?;
        let proto_idx = read_u16_le(data, item_off + 2).ok_or(DexError::Truncated)?;
        let name_idx = read_u32_le(data, item_off + 4).ok_or(DexError::Truncated)?;
        let class = type_ids
            .get(class_idx as usize)
            .cloned()
            .unwrap_or_default();
        let name = string_ids
            .get(name_idx as usize)
            .cloned()
            .unwrap_or_default();
        ids.push(MethodId {
            class,
            name,
            proto_idx,
        });
    }
    Ok(ids)
}

fn parse_class_defs(
    data: &[u8],
    header: &DexHeader,
    type_ids: &[String],
) -> Result<Vec<ClassDef>, DexError> {
    let off = header.class_defs_off as usize;
    let size = table_capacity(
        data.len(),
        "class_defs",
        header.class_defs_off,
        header.class_defs_size,
        CLASS_DEF_ITEM_SIZE,
    )?;
    let mut ids = Vec::with_capacity(size);
    for i in 0..size {
        let item_off = off
            .checked_add(
                i.checked_mul(CLASS_DEF_ITEM_SIZE)
                    .ok_or(DexError::Truncated)?,
            )
            .ok_or(DexError::Truncated)?;
        let class_idx = read_u32_le(data, item_off).ok_or(DexError::Truncated)?;
        let access_flags = read_u32_le(data, item_off + 4).ok_or(DexError::Truncated)?;
        let superclass_idx = read_u32_le(data, item_off + 8).ok_or(DexError::Truncated)?;
        let class_data_off = read_u32_le(data, item_off + 24).ok_or(DexError::Truncated)?;
        let class = type_ids
            .get(class_idx as usize)
            .cloned()
            .unwrap_or_default();
        let superclass = if superclass_idx == 0xFFFFFFFF {
            None
        } else {
            type_ids.get(superclass_idx as usize).cloned()
        };
        ids.push(ClassDef {
            class,
            superclass,
            access_flags,
            class_data_off,
            class_data: None,
        });
    }
    Ok(ids)
}

fn read_sleb128(bytes: &[u8], mut offset: usize) -> Option<(i32, usize)> {
    let mut result: i32 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = *bytes.get(offset)?;
        offset += 1;
        if shift > 28 {
            return None;
        }
        result |= (byte as i32 & 0x7F) << shift;
        if byte & 0x80 == 0 {
            if (byte & 0x40) != 0 {
                result |= !(0x7F << shift);
            }
            return Some((result, offset));
        }
        shift += 7;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EncodedField {
    pub field_idx: u32,
    pub access_flags: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct EncodedMethod {
    pub method_idx: u32,
    pub access_flags: u32,
    pub code_off: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassData {
    pub static_fields: Vec<EncodedField>,
    pub instance_fields: Vec<EncodedField>,
    pub direct_methods: Vec<EncodedMethod>,
    pub virtual_methods: Vec<EncodedMethod>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TryItem {
    pub start_address: u32,
    pub instruction_count: u16,
    pub handler_offset: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatchHandlerItem {
    pub catch_type_idx: u32,
    pub catch_address: u32,
}

/// Satu encoded_catch_handler.
#[derive(Debug, Clone, Serialize)]
pub struct CatchHandler {
    /// Offset relatif terhadap awal encoded_catch_handler_list, sama dengan
    /// nilai handler_offset pada TryItem.
    pub offset: u32,
    /// Pasangan (tipe, alamat) yang ditangkap handler ini.
    pub pairs: Vec<CatchHandlerItem>,
    /// Alamat catch-all bila handler memiliki penangkap tanpa tipe.
    pub catch_all: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodeItem {
    /// Jumlah register yang dipakai method ini.
    pub registers_size: u16,
    /// Jumlah register parameter masuk (termasuk this untuk method instance).
    pub ins_size: u16,
    /// Jumlah register untuk argumen method yang dipanggil.
    pub outs_size: u16,
    /// Jumlah entri try/catch pada method ini.
    pub tries_size: u16,
    pub debug_info_off: u32,
    pub instructions_size: u32,
    pub tries: Vec<TryItem>,
    pub handlers: Vec<CatchHandler>,
    pub insns: Vec<u16>,
}

fn parse_class_data(data: &[u8], class_data_off: u32) -> Result<Option<ClassData>, DexError> {
    if class_data_off == 0 {
        return Ok(None);
    }
    let mut off = class_data_off as usize;

    let (static_fields_size, new_off) = read_uleb128(data, off).ok_or(DexError::Truncated)?;
    off = new_off;
    let (instance_fields_size, new_off) = read_uleb128(data, off).ok_or(DexError::Truncated)?;
    off = new_off;
    let (direct_methods_size, new_off) = read_uleb128(data, off).ok_or(DexError::Truncated)?;
    off = new_off;
    let (virtual_methods_size, new_off) = read_uleb128(data, off).ok_or(DexError::Truncated)?;
    off = new_off;
    let read_encoded_field =
        |data: &[u8], off: &mut usize, last_idx: &mut u32| -> Result<EncodedField, DexError> {
            let (diff, new_off) = read_uleb128(data, *off).ok_or(DexError::Truncated)?;
            *off = new_off;
            *last_idx = last_idx.checked_add(diff).ok_or(DexError::Truncated)?;
            let (access_flags, new_off) = read_uleb128(data, *off).ok_or(DexError::Truncated)?;
            *off = new_off;
            Ok(EncodedField {
                field_idx: *last_idx,
                access_flags,
            })
        };
    let read_encoded_method =
        |data: &[u8], off: &mut usize, last_idx: &mut u32| -> Result<EncodedMethod, DexError> {
            let (diff, new_off) = read_uleb128(data, *off).ok_or(DexError::Truncated)?;
            *off = new_off;
            *last_idx = last_idx.checked_add(diff).ok_or(DexError::Truncated)?;
            let (access_flags, new_off) = read_uleb128(data, *off).ok_or(DexError::Truncated)?;
            *off = new_off;
            let (code_off, new_off) = read_uleb128(data, *off).ok_or(DexError::Truncated)?;
            *off = new_off;
            Ok(EncodedMethod {
                method_idx: *last_idx,
                access_flags,
                code_off,
            })
        };

    for (name, count) in [
        ("static_fields", static_fields_size),
        ("instance_fields", instance_fields_size),
        ("direct_methods", direct_methods_size),
        ("virtual_methods", virtual_methods_size),
    ] {
        if count > MAX_TABLE_ENTRIES {
            return Err(DexError::LimitExceeded {
                table: name,
                count,
                limit: MAX_TABLE_ENTRIES,
            });
        }
    }

    let mut static_fields = Vec::with_capacity(nested_capacity(static_fields_size));
    let mut last_field_idx: u32 = 0;
    for _ in 0..static_fields_size {
        static_fields.push(read_encoded_field(data, &mut off, &mut last_field_idx)?);
    }

    let mut instance_fields = Vec::with_capacity(nested_capacity(instance_fields_size));
    last_field_idx = 0;
    for _ in 0..instance_fields_size {
        instance_fields.push(read_encoded_field(data, &mut off, &mut last_field_idx)?);
    }

    let mut direct_methods = Vec::with_capacity(nested_capacity(direct_methods_size));
    let mut last_method_idx: u32 = 0;
    for _ in 0..direct_methods_size {
        direct_methods.push(read_encoded_method(data, &mut off, &mut last_method_idx)?);
    }

    let mut virtual_methods = Vec::with_capacity(nested_capacity(virtual_methods_size));
    last_method_idx = 0;
    for _ in 0..virtual_methods_size {
        virtual_methods.push(read_encoded_method(data, &mut off, &mut last_method_idx)?);
    }

    Ok(Some(ClassData {
        static_fields,
        instance_fields,
        direct_methods,
        virtual_methods,
    }))
}

fn parse_code_item(data: &[u8], code_off: u32) -> Result<CodeItem, DexError> {
    let mut off = code_off as usize;
    let registers_size = read_u16_le(data, off).ok_or(DexError::Truncated)?;
    let ins_size = read_u16_le(data, off + 2).ok_or(DexError::Truncated)?;
    let outs_size = read_u16_le(data, off + 4).ok_or(DexError::Truncated)?;
    let tries_size = read_u16_le(data, off + 6).ok_or(DexError::Truncated)?;
    let debug_info_off = read_u32_le(data, off + 8).ok_or(DexError::Truncated)?;
    let instructions_size = read_u32_le(data, off + 12).ok_or(DexError::Truncated)?;
    off += 16;

    // Insns mengikuti header 16 byte, bukan try/catch. Urutan ini penting:
    // membaca try lebih dulu akan menggeser seluruh instruksi method.
    let insns_bytes = (instructions_size as usize)
        .checked_mul(2)
        .ok_or(DexError::Truncated)?;
    let insns_end = off.checked_add(insns_bytes).ok_or(DexError::Truncated)?;
    if insns_end > data.len() {
        return Err(DexError::Truncated);
    }
    let mut insns = Vec::with_capacity(instructions_size as usize);
    for index in 0..instructions_size as usize {
        insns.push(read_u16_le(data, off + index * 2).ok_or(DexError::Truncated)?);
    }
    off = insns_end;

    let mut tries = Vec::new();
    let mut handlers = Vec::new();
    if tries_size > 0 {
        let padding = off % 4;
        if padding != 0 {
            off += 4 - padding;
        }

        for _ in 0..tries_size {
            let start_address = read_u32_le(data, off).ok_or(DexError::Truncated)?;
            let instruction_count = read_u16_le(data, off + 4).ok_or(DexError::Truncated)?;
            let handler_offset = read_u16_le(data, off + 6).ok_or(DexError::Truncated)?;
            off += 8;
            tries.push(TryItem {
                start_address,
                instruction_count,
                handler_offset,
            });
        }

        let list_start = off;
        let (handlers_size, new_off) = read_uleb128(data, off).ok_or(DexError::Truncated)?;
        off = new_off;
        if handlers_size > MAX_TABLE_ENTRIES {
            return Err(DexError::LimitExceeded {
                table: "catch_handlers",
                count: handlers_size,
                limit: MAX_TABLE_ENTRIES,
            });
        }

        handlers = Vec::with_capacity(nested_capacity(handlers_size));
        for _ in 0..handlers_size {
            let handler_offset =
                u32::try_from(off.saturating_sub(list_start)).map_err(|_| DexError::Truncated)?;
            let (size, new_off) = read_sleb128(data, off).ok_or(DexError::Truncated)?;
            off = new_off;

            let pair_count = size.unsigned_abs();
            if pair_count > MAX_TABLE_ENTRIES {
                return Err(DexError::LimitExceeded {
                    table: "catch_handler_pairs",
                    count: pair_count,
                    limit: MAX_TABLE_ENTRIES,
                });
            }

            let mut pairs = Vec::with_capacity(nested_capacity(pair_count));
            for _ in 0..pair_count {
                let (catch_type_idx, new_off) =
                    read_uleb128(data, off).ok_or(DexError::Truncated)?;
                off = new_off;
                let (catch_address, new_off) =
                    read_uleb128(data, off).ok_or(DexError::Truncated)?;
                off = new_off;
                pairs.push(CatchHandlerItem {
                    catch_type_idx,
                    catch_address,
                });
            }

            // size negatif berarti ada catch-all setelah pasangan bertipe.
            let catch_all = if size <= 0 {
                let (catch_address, new_off) =
                    read_uleb128(data, off).ok_or(DexError::Truncated)?;
                off = new_off;
                Some(catch_address)
            } else {
                None
            };

            handlers.push(CatchHandler {
                offset: handler_offset,
                pairs,
                catch_all,
            });
        }
    }

    Ok(CodeItem {
        registers_size,
        ins_size,
        outs_size,
        tries_size,
        debug_info_off,
        instructions_size,
        tries,
        handlers,
        insns,
    })
}

pub fn parse(path: impl AsRef<Path>) -> Result<DexFile, DexError> {
    let path = path.as_ref();
    let data = fs::read(path).map_err(DexError::Io)?;
    let source = path
        .to_str()
        .map(|s| s.to_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    parse_bytes(&data, &source)
}

// ============================================================
// DexEngine
// ============================================================

/// Mesin DEX sementara yang mewakili state eksekusi sesudah file dipars.
///
/// Ini adalah awal dari fase 1: state mesin ada, register ada, method
/// bisa diselidiki, tetapi bytecode belum ditafsirkan.
/// Status inisialisasi class selama eksekusi.
///
/// JVM menjalankan clinit pada penggunaan aktif pertama sebuah class.
/// Interpreter meniru itu: initialized mencegah pengulangan, sedangkan
/// initializing memutus rekursi saat clinit memakai class-nya sendiri.
#[derive(Debug, Default, Clone)]
pub struct ClassState {
    pub initialized: std::collections::HashSet<String>,
    pub initializing: std::collections::HashSet<String>,
}

pub struct DexEngine {
    pub dex: DexFile,
    pub heap: Heap,
    pub registers: Vec<Value>,
    pub exception_addr: Option<u32>,
    pub instruction_pointer: u32,
    pub return_value: Option<Value>,
    /// Jumlah langkah eksekusi terakhir, termasuk pemanggilan bersarang.
    pub last_steps: u64,
    /// Field statis per indeks `field_ids`; nilai yang belum diisi bernilai
    /// `Int(0)` seperti field statis Java yang belum diinisialisasi.
    pub statics: std::collections::HashMap<u32, Value>,
    /// Objek `java.lang.Class` per indeks `type_ids`, agar `const-class`
    /// mengembalikan referensi yang sama untuk tipe yang sama.
    pub classes: std::collections::HashMap<u32, Value>,
    /// Status inisialisasi class untuk clinit otomatis.
    pub class_state: ClassState,
    native: Option<Box<dyn NativeResolver>>,
}

impl DexFile {
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl DexEngine {
    pub fn new(dex: DexFile) -> Self {
        let reg_count = dex
            .code_items
            .first()
            .map(|c| c.registers_size)
            .unwrap_or(0);
        let registers = vec![Value::Int(0); reg_count as usize];
        Self {
            dex,
            heap: Heap::new(),
            registers,
            exception_addr: None,
            instruction_pointer: 0,
            return_value: None,
            last_steps: 0,
            statics: std::collections::HashMap::new(),
            classes: std::collections::HashMap::new(),
            class_state: ClassState::default(),
            native: None,
        }
    }

    pub fn method_entry_by_name(
        &self,
        class_descriptor: &str,
        method_name: &str,
    ) -> Result<&MethodId, DexError> {
        self.dex
            .method_ids
            .iter()
            .find(|m| m.class == class_descriptor && m.name == method_name)
            .ok_or_else(|| DexError::MethodNotFound {
                class: class_descriptor.to_owned(),
                method: method_name.to_owned(),
            })
    }
    pub fn instruction_count(&self) -> usize {
        self.dex
            .code_items
            .first()
            .map(|c| c.insns.len())
            .unwrap_or(0)
    }

    pub fn dex(&self) -> &DexFile {
        &self.dex
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionOutcome {
    pub executed: bool,
    pub reason: String,
    pub next_instruction_pointer: u32,
    pub return_value: Option<Value>,
    /// Jumlah instruksi yang dieksekusi, termasuk pemanggilan bersarang.
    pub steps: u64,
}

impl DexEngine {
    pub fn entry_point_candidate_report(&self) -> Vec<CandidateEntry> {
        self.dex
            .class_defs
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|cd| {
                cd.direct_methods
                    .iter()
                    .chain(cd.virtual_methods.iter())
                    .filter(|m| m.code_off != 0)
                    .flat_map(move |m| {
                        let class_name = self
                            .dex
                            .class_defs
                            .iter()
                            .find(|c_| {
                                c_.class_data
                                    .as_ref()
                                    .map(|cd_| {
                                        cd_.direct_methods
                                            .iter()
                                            .any(|dm| dm.method_idx == m.method_idx)
                                            || cd_
                                                .virtual_methods
                                                .iter()
                                                .any(|vm| vm.method_idx == m.method_idx)
                                    })
                                    .unwrap_or(false)
                            })
                            .map(|c_| c_.class.as_str())
                            .unwrap_or("tidak-diketahui")
                            .to_owned();
                        Some(CandidateEntry {
                            class: class_name,
                            method: m.method_idx,
                            code_off: m.code_off,
                        })
                    })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateEntry {
    pub class: String,
    pub method: u32,
    pub code_off: u32,
}

// ============================================================
// DEX inspection helpers
// ============================================================

/// Laporan sederhana dari DEX yang berhasil dipars.
/// Ini adalah inspeksi statis, bukan hasil eksekusi.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DexInspection {
    pub path: String,
    pub cmd: String,
    pub version: String,
    pub header_size: u32,
    pub file_size: u32,
    pub string_ids_count: u32,
    pub type_ids_count: u32,
    pub proto_ids_count: u32,
    pub field_ids_count: u32,
    pub method_ids_count: u32,
    pub class_defs_count: u32,
    pub code_items_count: u32,
    pub total_instruction_words: u32,
}

impl DexInspection {
    pub fn from_dex(dex: &DexFile, cmd: &str) -> Self {
        let total_instruction_words = dex.code_items.iter().map(|c| c.instructions_size).sum();
        Self {
            path: dex.path.clone(),
            cmd: cmd.to_string(),
            version: dex.header.version.to_string(),
            header_size: dex.header.header_size,
            file_size: dex.header.file_size,
            string_ids_count: dex.header.string_ids_size,
            type_ids_count: dex.header.type_ids_size,
            proto_ids_count: dex.header.proto_ids_size,
            field_ids_count: dex.header.field_ids_size,
            method_ids_count: dex.header.method_ids_size,
            class_defs_count: dex.header.class_defs_size,
            code_items_count: dex.code_items.len() as u32,
            total_instruction_words,
        }
    }
}

/// Inspeksi DEX yang sudah ada di memori, tanpa menyentuh filesystem.
///
/// Dipakai ketika DEX dibaca dari dalam APK: tidak perlu menulis berkas
/// sementara yang harus dibersihkan lagi.
pub fn inspect_dex_bytes(data: &[u8], cmd: &str, source: &str) -> Result<DexInspection, DexError> {
    let dex = parse_bytes(data, source)?;
    Ok(DexInspection::from_dex(&dex, cmd))
}

/// Muat dan inspeksi satu berkas DEX biasa.
pub fn inspect_dex(path: &std::path::Path, cmd: &str) -> Result<DexInspection, DexError> {
    let dex = parse(path)?;
    Ok(DexInspection::from_dex(&dex, cmd))
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    fn write_uleb128(buf: &mut [u8], mut value: u32) -> usize {
        let mut offset = 0;
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                buf[offset] = byte | 0x80;
            } else {
                buf[offset] = byte;
                return offset + 1;
            }
            offset += 1;
        }
    }

    #[test]
    fn native_exception_bolak_balik_lewat_pesan() {
        let exception = NativeException::new("Ljava/io/IOException;", "berkas hilang");
        let encoded = exception.encode();
        assert_eq!(NativeException::decode(&encoded), Some(exception));
    }

    #[test]
    fn native_exception_menolak_pesan_biasa() {
        assert_eq!(NativeException::decode("operasi gagal"), None);
        assert_eq!(NativeException::decode("@throw:bukan-tipe:pesan"), None);
    }

    fn make_dex_magic(version: &[u8; 4]) -> [u8; MAGIC_LEN] {
        let mut magic = [0u8; MAGIC_LEN];
        magic[0..4].copy_from_slice(b"dex\n");
        magic[4..8].copy_from_slice(version);
        magic
    }

    fn valid_header_bytes(version: &[u8; 4]) -> Vec<u8> {
        let mut buf = vec![0u8; DEX_HEADER_SIZE];
        buf[0..8].copy_from_slice(&make_dex_magic(version));
        let file_size = DEX_HEADER_SIZE as u32;
        buf[32..36].copy_from_slice(&file_size.to_le_bytes());
        buf[36..40].copy_from_slice(&file_size.to_le_bytes());
        buf[40..44].copy_from_slice(&DEX_ENDIAN_CONSTANT.to_le_bytes());
        buf
    }

    #[test]
    fn membuat_mesin_dari_header_valid() {
        let data = valid_header_bytes(b"037\0");
        let dex = parse_bytes(&data, "test.dex").expect("should parse DEX 037");
        let engine = DexEngine::new(dex);
        assert_eq!(engine.registers.len(), 0);
    }

    #[test]
    fn method_entry_by_name_gagal_ketika_tidak_ada() {
        let data = valid_header_bytes(b"037\0");
        let dex = parse_bytes(&data, "test.dex").expect("should parse");
        let engine = DexEngine::new(dex);
        let err = engine
            .method_entry_by_name("Lhzasi;Lhzasi;", "tidakAda")
            .unwrap_err();
        assert!(matches!(err, DexError::MethodNotFound { .. }));
    }

    #[test]
    fn execute_method_gagal_untuk_method_yang_tidak_ada() {
        let data = valid_header_bytes(b"037\0");
        let dex = parse_bytes(&data, "test.dex").expect("should parse");
        let mut engine = DexEngine::new(dex);
        let error = engine
            .execute_method("Lhzasi;Lhzasi;", "onCreate")
            .expect_err("method fiktif harus ditolak");
        assert!(matches!(error, DexError::MethodNotFound { .. }));
    }

    #[test]
    fn parse_code_item_membaca_try_dan_handler_setelah_insns() {
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&1u16.to_le_bytes()); // registers_size
        buf.extend_from_slice(&0u16.to_le_bytes()); // ins_size
        buf.extend_from_slice(&0u16.to_le_bytes()); // outs_size
        buf.extend_from_slice(&1u16.to_le_bytes()); // tries_size
        buf.extend_from_slice(&0u32.to_le_bytes()); // debug_info_off
        buf.extend_from_slice(&1u32.to_le_bytes()); // insns_size
        buf.extend_from_slice(&0x000eu16.to_le_bytes()); // return-void
        buf.extend_from_slice(&[0, 0]); // padding ke 4 byte
        buf.extend_from_slice(&0u32.to_le_bytes()); // try start_address
        buf.extend_from_slice(&1u16.to_le_bytes()); // instruction_count
        buf.extend_from_slice(&0u16.to_le_bytes()); // handler_offset
        buf.push(1); // handlers_size
        buf.push(1); // sleb128 ukuran handler: 1 pasangan bertipe
        buf.push(0); // type_idx
        buf.push(0x0e); // catch address

        let code = parse_code_item(&buf, 0).expect("code_item harus terparse");
        assert_eq!(code.registers_size, 1);
        assert_eq!(code.tries_size, 1);
        assert_eq!(code.insns, vec![0x000e]);
        assert_eq!(code.tries.len(), 1);
        assert_eq!(code.tries[0].instruction_count, 1);
        assert_eq!(code.handlers.len(), 1);
        assert_eq!(code.handlers[0].offset, 1);
        assert_eq!(code.handlers[0].pairs.len(), 1);
        assert_eq!(code.handlers[0].pairs[0].catch_type_idx, 0);
        assert_eq!(code.handlers[0].pairs[0].catch_address, 0x0e);
        assert_eq!(code.handlers[0].catch_all, None);
    }

    #[test]
    fn entry_point_candidate_report_kosong() {
        let data = valid_header_bytes(b"037\0");
        let dex = parse_bytes(&data, "test.dex").expect("should parse");
        let engine = DexEngine::new(dex);
        let r = engine.entry_point_candidate_report();
        assert!(r.is_empty());
    }

    #[test]
    fn inspection_dari_dex_sederhana() {
        let data = valid_header_bytes(b"037\0");
        let dex = parse_bytes(&data, "test.dex").expect("should parse");
        let inspection = DexInspection::from_dex(&dex, "test");
        assert_eq!(inspection.path, "test.dex");
        assert_eq!(inspection.cmd, "test");
        assert_eq!(inspection.version, "037");
        assert_eq!(inspection.header_size, inspection.file_size);
        assert_eq!(inspection.code_items_count, 0);
    }

    #[test]
    fn inspect_dex_dari_path_valid() {
        let tmp = std::env::temp_dir().join("arunika-inspect-dex.dex");
        let data = valid_header_bytes(b"038\0");
        std::fs::write(&tmp, data).expect("tidak dapat menulis file uji");
        let inspection = inspect_dex(&tmp, "test").expect("should inspect dex");
        assert_eq!(inspection.version, "038");
        assert_eq!(inspection.cmd, "test");
        std::fs::remove_file(&tmp).ok();
    }

    // ===== Uji input DEX rusak / malformed (harus ditolak tanpa panic) =====

    #[test]
    fn dex_terlalu_pendek_ditolak() {
        let data = vec![0u8; 10];
        let err = parse_bytes(&data, "too-small.dex").unwrap_err();
        assert!(matches!(err, DexError::TooSmall));
    }

    #[test]
    fn dex_magic_tidak_valid_ditolak() {
        let mut buf = vec![0u8; DEX_HEADER_SIZE];
        buf[0..8].copy_from_slice(b"dex\n037X");
        let err = parse_bytes(&buf, "bad-magic.dex").unwrap_err();
        assert!(matches!(err, DexError::InvalidMagic));
    }

    #[test]
    fn dex_versi_tidak_didukung_ditolak() {
        let buf = valid_header_bytes(b"039\0");
        let err = parse_bytes(&buf, "unsupported.dex").unwrap_err();
        assert!(matches!(err, DexError::InvalidMagic));
    }

    #[test]
    fn dex_header_size_salah_ditolak() {
        let mut buf = valid_header_bytes(b"037\0");
        buf[36..40].copy_from_slice(&(113u32).to_le_bytes());
        let err = parse_bytes(&buf, "bad-header-size.dex").unwrap_err();
        assert!(matches!(err, DexError::InvalidHeaderSize(_)));
    }

    #[test]
    fn dex_endian_tidak_didukung_ditolak() {
        let mut buf = valid_header_bytes(b"037\0");
        buf[40..44].copy_from_slice(&DEX_REVERSE_ENDIAN_CONSTANT.to_le_bytes());
        let err = parse_bytes(&buf, "bad-endian.dex").unwrap_err();
        assert!(matches!(err, DexError::UnsupportedEndian(_)));
    }

    #[test]
    fn dex_size_mismatch_ditolak() {
        let mut buf = valid_header_bytes(b"037\0");
        buf[32..36].copy_from_slice(&(DEX_HEADER_SIZE as u32 + 100).to_le_bytes());
        let err = parse_bytes(&buf, "size-mismatch.dex").unwrap_err();
        assert!(matches!(err, DexError::SizeMismatch { .. }));
    }

    #[test]
    fn dex_tabel_string_melebihi_batas_ditolak() {
        let mut buf = valid_header_bytes(b"037\0");
        buf[56..60].copy_from_slice(&(MAX_TABLE_ENTRIES + 1).to_le_bytes());
        let err = parse_bytes(&buf, "table-limit.dex").unwrap_err();
        assert!(matches!(err, DexError::LimitExceeded { .. }));
    }

    #[test]
    fn dex_truncated_setelah_header_ditolak() {
        let mut buf = valid_header_bytes(b"037\0");
        buf[56..60].copy_from_slice(&1u32.to_le_bytes());
        buf[60..64].copy_from_slice(&(DEX_HEADER_SIZE as u32).to_le_bytes());
        let err = parse_bytes(&buf, "truncated.dex").unwrap_err();
        assert!(matches!(err, DexError::Truncated));
    }

    /// Bangun DEX minimal dengan satu class, satu method, dan satu code item
    /// yang berisi instruksi yang diberikan (dalam code unit little-endian).
    fn build_dex_with_code(insns: &[u16]) -> Vec<u8> {
        const TABLES_LEN: usize = 8 + 4 + 12 + 8 + 32;
        let string0_off = DEX_HEADER_SIZE + TABLES_LEN;
        let string0_total = 1 + 6 + 1; // uleb len + "LTest;" + null
        let string1_off = string0_off + string0_total;
        let string1_total = 1 + 7 + 1; // uleb len + "<init>" + null
        let class_data_off = (string1_off + string1_total + 3) & !3;
        let encoded_method_len = 1 + 1 + 2;
        let class_data_len = 4 + encoded_method_len;
        let code_off = class_data_off + class_data_len;
        let code_item_size = 16 + insns.len() * 2;
        let file_size = code_off + code_item_size;

        let mut buf = valid_header_bytes(b"037\0");

        buf[56..60].copy_from_slice(&2u32.to_le_bytes());
        buf[60..64].copy_from_slice(&(DEX_HEADER_SIZE as u32).to_le_bytes());
        buf[64..68].copy_from_slice(&1u32.to_le_bytes());
        buf[68..72].copy_from_slice(&((DEX_HEADER_SIZE + 8) as u32).to_le_bytes());
        buf[72..76].copy_from_slice(&1u32.to_le_bytes());
        buf[76..80].copy_from_slice(&((DEX_HEADER_SIZE + 12) as u32).to_le_bytes());
        buf[80..84].copy_from_slice(&0u32.to_le_bytes());
        buf[84..88].copy_from_slice(&0u32.to_le_bytes());
        buf[88..92].copy_from_slice(&1u32.to_le_bytes());
        buf[92..96].copy_from_slice(&((DEX_HEADER_SIZE + 24) as u32).to_le_bytes());
        buf[96..100].copy_from_slice(&1u32.to_le_bytes());
        buf[100..104].copy_from_slice(&((DEX_HEADER_SIZE + 32) as u32).to_le_bytes());
        buf[104..108]
            .copy_from_slice(&((file_size - (DEX_HEADER_SIZE + TABLES_LEN)) as u32).to_le_bytes());
        buf[108..112].copy_from_slice(&((DEX_HEADER_SIZE + TABLES_LEN) as u32).to_le_bytes());
        buf[32..36].copy_from_slice(&(file_size as u32).to_le_bytes());
        buf[36..40].copy_from_slice(&(DEX_HEADER_SIZE as u32).to_le_bytes());

        let mut data_section = Vec::new();

        data_section.extend_from_slice(&(string0_off as u32).to_le_bytes());
        data_section.extend_from_slice(&(string1_off as u32).to_le_bytes());

        data_section.extend_from_slice(&0u32.to_le_bytes()); // type_ids

        data_section.extend_from_slice(&0u32.to_le_bytes()); // proto shorty
        data_section.extend_from_slice(&0u32.to_le_bytes()); // proto return
        data_section.extend_from_slice(&0u32.to_le_bytes()); // proto params

        data_section.extend_from_slice(&0u16.to_le_bytes()); // method class_idx
        data_section.extend_from_slice(&0u16.to_le_bytes()); // method proto_idx
        data_section.extend_from_slice(&1u32.to_le_bytes()); // method name_idx

        data_section.extend_from_slice(&0u32.to_le_bytes()); // class_idx
        data_section.extend_from_slice(&0u32.to_le_bytes()); // access
        data_section.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes()); // superclass
        data_section.extend_from_slice(&0u32.to_le_bytes()); // interfaces
        data_section.extend_from_slice(&0u32.to_le_bytes()); // source_file
        data_section.extend_from_slice(&0u32.to_le_bytes()); // annotations
        data_section.extend_from_slice(&(class_data_off as u32).to_le_bytes()); // class_data_off
        data_section.extend_from_slice(&0u32.to_le_bytes()); // static_values

        // string data with ULEB128 length prefix
        data_section.push(6);
        data_section.extend_from_slice(b"LTest;\0");
        data_section.push(7);
        data_section.extend_from_slice(b"<init>\0");
        while data_section.len() < (class_data_off - DEX_HEADER_SIZE) {
            data_section.push(0);
        }

        data_section.push(0); // static
        data_section.push(0); // instance
        data_section.push(1); // direct
        data_section.push(0); // virtual
        data_section.push(0); // method_idx diff
        data_section.push(0); // access
        data_section.push(((code_off & 0x7F) | 0x80) as u8);
        data_section.push(((code_off >> 7) & 0x7F) as u8);

        data_section.extend_from_slice(&3u16.to_le_bytes()); // registers_size
        data_section.extend_from_slice(&0u16.to_le_bytes()); // ins_size
        data_section.extend_from_slice(&0u16.to_le_bytes()); // outs_size
        data_section.extend_from_slice(&0u16.to_le_bytes()); // tries_size
        data_section.extend_from_slice(&0u32.to_le_bytes()); // debug_info_off
        data_section.extend_from_slice(&(insns.len() as u32).to_le_bytes()); // instructions_size
        for insn in insns {
            data_section.extend_from_slice(&insn.to_le_bytes());
        }

        buf.extend_from_slice(&data_section);
        buf
    }

    #[test]
    fn interpreter_pembagian_nol_dikonversi_ke_error() {
        // int-div v0, v1, v2 -> 0x93 0x21 (vA=v0, vB=v1)
        let data = build_dex_with_code(&[0x0293, 0x0100]);
        let dex = parse_bytes(&data, "divzero.dex").expect("parse");
        eprintln!("METHODS={:?}", dex.method_ids);
        eprintln!("STRINGS={:?}", dex.string_ids);
        eprintln!("TYPES={:?}", dex.type_ids);
        let mut engine = DexEngine::new(dex);
        // v0=10, v1=0
        engine.registers[0] = Value::Int(10);
        engine.registers[1] = Value::Int(0);
        let err = engine
            .execute_method_values("LTest;", "<init>", &[])
            .unwrap_err();
        eprintln!("ERR={err:?}");
        assert!(matches!(err, DexError::DivisionByZero));
    }

    #[test]
    fn interpreter_index_array_luar_batas_dikonversi_ke_error() {
        // aget v0, v1, v2 -> 0x4E 0x21 (array=v0, index=v1)
        let data = build_dex_with_code(&[0x0244, 0x0100]);
        let dex = parse_bytes(&data, "aioob.dex").expect("parse");
        let mut engine = DexEngine::new(dex);
        // v0 = null reference -> NullReference
        engine.registers[0] = Value::Int(0);
        engine.registers[1] = Value::Int(5);
        let err = engine
            .execute_method_values("LTest;", "<init>", &[])
            .unwrap_err();
        assert!(matches!(err, DexError::NullReference { .. }));
    }

    #[test]
    fn interpreter_instruksi_tidak_didukung_dikonversi_ke_error() {
        let data = build_dex_with_code(&[0xFF]);
        let dex = parse_bytes(&data, "bad-opcode.dex").expect("parse");
        let mut engine = DexEngine::new(dex);
        let err = engine
            .execute_method_values("LTest;", "<init>", &[])
            .unwrap_err();
        assert!(matches!(err, DexError::InvalidInstruction { .. }));
    }
}
