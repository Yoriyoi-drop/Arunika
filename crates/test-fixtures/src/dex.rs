//! Pembangkit berkas DEX minimal untuk pengujian parser dan interpreter.
//!
//! Berkas yang dihasilkan valid secara struktur untuk parser Arunika, tetapi
//! tidak dijamin lolos verifikasi ART: string dan type id tidak diurutkan dan
//! checksum/signature dibiarkan nol. Tujuannya adalah menguji semantik
//! instruksi, bukan menjalankan berkas di perangkat Android.

use std::collections::HashMap;

pub const NO_INDEX: u32 = 0xFFFF_FFFF;
pub const ACC_PUBLIC: u32 = 0x0001;
pub const ACC_PRIVATE: u32 = 0x0002;
pub const ACC_STATIC: u32 = 0x0008;
pub const ACC_CONSTRUCTOR: u32 = 0x1_0000;

const HEADER_SIZE: u32 = 112;

#[derive(Debug, Clone)]
pub struct MethodSpec {
    pub name: String,
    pub access_flags: u32,
    pub params: Vec<String>,
    pub ret: String,
    pub registers: u16,
    pub ins: u16,
    pub outs: u16,
    pub insns: Vec<u16>,
    /// String literal yang perlu masuk string pool (untuk const-string).
    pub literals: Vec<String>,
    /// Daftar handler yang dapat dirujuk oleh `tries`.
    pub handlers: Vec<CatchSpec>,
    /// Rentang instruksi yang dilindungi try/catch.
    pub tries: Vec<TrySpec>,
}

/// Satu encoded_catch_handler pada fixture.
#[derive(Debug, Clone)]
pub struct CatchSpec {
    /// Pasangan (descriptor tipe, alamat handler).
    pub pairs: Vec<(String, u32)>,
    /// Alamat catch-all bila ada.
    pub catch_all: Option<u32>,
}

impl CatchSpec {
    /// Handler yang hanya menangkap satu tipe.
    pub fn typed(descriptor: &str, address: u32) -> Self {
        Self {
            pairs: vec![(descriptor.to_string(), address)],
            catch_all: None,
        }
    }

    /// Handler tanpa tipe: menangkap semua eksepsi.
    pub fn catch_all(address: u32) -> Self {
        Self {
            pairs: Vec::new(),
            catch_all: Some(address),
        }
    }
}

/// Satu try_item pada fixture.
#[derive(Debug, Clone)]
pub struct TrySpec {
    pub start: u32,
    pub count: u16,
    /// Indeks ke `MethodSpec::handlers`.
    pub handler: usize,
}

impl TrySpec {
    pub fn new(start: u32, count: u16, handler: usize) -> Self {
        Self {
            start,
            count,
            handler,
        }
    }
}

impl MethodSpec {
    pub fn new(name: &str, access_flags: u32, params: &[&str], ret: &str) -> Self {
        Self {
            name: name.to_string(),
            access_flags,
            params: params.iter().map(|p| p.to_string()).collect(),
            ret: ret.to_string(),
            registers: 0,
            ins: 0,
            outs: 0,
            insns: Vec::new(),
            literals: Vec::new(),
            handlers: Vec::new(),
            tries: Vec::new(),
        }
    }

    /// Tambahkan satu handler dan satu try_item yang merujuknya.
    pub fn try_catch(mut self, start: u32, count: u16, handler: CatchSpec) -> Self {
        let index = self.handlers.len();
        self.handlers.push(handler);
        self.tries.push(TrySpec::new(start, count, index));
        self
    }

    /// Daftarkan string literal agar masuk string pool.
    pub fn literal(mut self, text: &str) -> Self {
        self.literals.push(text.to_string());
        self
    }

    pub fn registers(mut self, registers: u16) -> Self {
        self.registers = registers;
        self
    }

    pub fn ins(mut self, ins: u16) -> Self {
        self.ins = ins;
        self
    }

    pub fn outs(mut self, outs: u16) -> Self {
        self.outs = outs;
        self
    }

    pub fn body(mut self, insns: Vec<u16>) -> Self {
        self.insns = insns;
        self
    }

    fn is_direct(&self) -> bool {
        self.access_flags & (ACC_STATIC | ACC_PRIVATE | ACC_CONSTRUCTOR) != 0
    }
}

/// Deklarasi satu field pada fixture.
#[derive(Debug, Clone)]
pub struct FieldSpec {
    pub name: String,
    pub field_type: String,
    pub access_flags: u32,
    pub is_static: bool,
}

impl FieldSpec {
    pub fn new(name: &str, field_type: &str, is_static: bool) -> Self {
        Self {
            name: name.to_string(),
            field_type: field_type.to_string(),
            access_flags: ACC_PUBLIC | if is_static { ACC_STATIC } else { 0 },
            is_static,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClassSpec {
    pub name: String,
    pub superclass: Option<String>,
    pub access_flags: u32,
    pub methods: Vec<MethodSpec>,
    pub fields: Vec<FieldSpec>,
}

impl ClassSpec {
    pub fn new(name: &str, superclass: Option<&str>) -> Self {
        Self {
            name: name.to_string(),
            superclass: superclass.map(|s| s.to_string()),
            access_flags: ACC_PUBLIC,
            methods: Vec::new(),
            fields: Vec::new(),
        }
    }

    pub fn with_method(mut self, method: MethodSpec) -> Self {
        self.methods.push(method);
        self
    }

    pub fn with_field(mut self, field: FieldSpec) -> Self {
        self.fields.push(field);
        self
    }
}

#[derive(Default)]
struct Pool {
    order: Vec<String>,
    index: HashMap<String, u32>,
}

impl Pool {
    fn intern(&mut self, value: &str) -> u32 {
        if let Some(&idx) = self.index.get(value) {
            return idx;
        }
        let idx = self.order.len() as u32;
        self.order.push(value.to_string());
        self.index.insert(value.to_string(), idx);
        idx
    }
}

fn shorty(descriptor: &str) -> String {
    match descriptor.as_bytes().first() {
        Some(b'L') | Some(b'[') => "L".to_string(),
        Some(_) => descriptor.chars().next().unwrap().to_string(),
        None => "V".to_string(),
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_uleb128(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn push_sleb128(out: &mut Vec<u8>, mut value: i32) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        let sign_bit = byte & 0x40 != 0;
        if (value == 0 && !sign_bit) || (value == -1 && sign_bit) {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Encode satu string DEX (MUTF-8) dengan panjang uleb128.
fn push_string_data(out: &mut Vec<u8>, value: &str) {
    let units: Vec<u16> = value.encode_utf16().collect();
    push_uleb128(out, units.len() as u32);
    for unit in units {
        let cp = u32::from(unit);
        if cp == 0 {
            out.push(0xC0);
            out.push(0x80);
        } else if cp < 0x80 {
            out.push(cp as u8);
        } else if cp < 0x800 {
            out.push(0xC0 | (cp >> 6) as u8);
            out.push(0x80 | (cp & 0x3F) as u8);
        } else {
            out.push(0xE0 | (cp >> 12) as u8);
            out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
            out.push(0x80 | (cp & 0x3F) as u8);
        }
    }
    out.push(0);
}

fn align4(data: &mut Vec<u8>) {
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
}

/// Hasil pembangunan DEX beserta indeks method untuk menyusun instruksi
/// invoke di dalam tes.
#[derive(Debug, Clone)]
pub struct DexBuild {
    pub bytes: Vec<u8>,
    pub method_indices: HashMap<(String, String), u16>,
    pub field_indices: HashMap<(String, String), u16>,
    pub string_indices: HashMap<String, u16>,
    pub type_indices: HashMap<String, u16>,
}

impl DexBuild {
    /// Indeks method_ids untuk pasangan class/method.
    pub fn method_index(&self, class: &str, method: &str) -> u16 {
        *self
            .method_indices
            .get(&(class.to_string(), method.to_string()))
            .unwrap_or_else(|| panic!("method {class}#{method} tidak ada di fixture"))
    }

    /// Indeks field_ids untuk pasangan class/field.
    pub fn field_index(&self, class: &str, field: &str) -> u16 {
        *self
            .field_indices
            .get(&(class.to_string(), field.to_string()))
            .unwrap_or_else(|| panic!("field {class}->{field} tidak ada di fixture"))
    }

    /// Indeks string_ids untuk teks tertentu.
    pub fn string_index(&self, text: &str) -> u16 {
        *self
            .string_indices
            .get(text)
            .unwrap_or_else(|| panic!("string {text} tidak ada di fixture"))
    }

    /// Indeks type_ids untuk descriptor tertentu.
    pub fn type_index(&self, descriptor: &str) -> u16 {
        *self
            .type_indices
            .get(descriptor)
            .unwrap_or_else(|| panic!("type {descriptor} tidak ada di fixture"))
    }
}

/// Bangun berkas DEX dari daftar class.
pub fn build_dex(classes: &[ClassSpec]) -> Vec<u8> {
    build_dex_with_symbols(classes).bytes
}

/// Bangun DEX sekaligus peta nama method ke indeks method_ids.
pub fn build_dex_with_symbols(classes: &[ClassSpec]) -> DexBuild {
    let mut strings = Pool::default();
    let mut types = Pool::default();
    let mut protos: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut proto_index: HashMap<(String, Vec<String>), u32> = HashMap::new();

    // Kumpulkan string, type, dan proto dalam urutan deterministik.
    for class in classes {
        types.intern(&class.name);
        if let Some(superclass) = &class.superclass {
            types.intern(superclass);
        }
        for field in &class.fields {
            strings.intern(&field.name);
            types.intern(&field.field_type);
        }
        for method in &class.methods {
            strings.intern(&method.name);
            for literal in &method.literals {
                strings.intern(literal);
            }
            for param in &method.params {
                types.intern(param);
            }
            for handler in &method.handlers {
                for (descriptor, _) in &handler.pairs {
                    types.intern(descriptor);
                }
            }
            types.intern(&method.ret);
        }
    }

    for class in classes {
        for method in &class.methods {
            let key = (method.ret.clone(), method.params.clone());
            if proto_index.contains_key(&key) {
                continue;
            }
            let shorty: String =
                shorty(&method.ret) + &method.params.iter().map(|p| shorty(p)).collect::<String>();
            let idx = protos.len() as u32;
            protos.push((shorty, method.ret.clone(), method.params.clone()));
            proto_index.insert(key, idx);
        }
    }

    // type_ids menunjuk ke string index, jadi setiap descriptor juga harus
    // menjadi entri string.
    for descriptor in &types.order {
        strings.intern(descriptor);
    }

    for proto in &protos {
        strings.intern(&proto.0);
    }

    let type_idx = |name: &str| *types.index.get(name).expect("type harus terdaftar");
    let string_idx = |name: &str| *strings.index.get(name).expect("string harus terdaftar");

    // method_ids harus terurut (class_idx, name_idx, proto_idx) agar encoding
    // class_data memakai diff yang valid.
    struct MethodEntry<'a> {
        class_idx: u32,
        proto_idx: u32,
        name_idx: u32,
        spec: &'a MethodSpec,
        method_idx: u32,
    }

    let mut entries: Vec<MethodEntry> = Vec::new();
    for (class_pos, class) in classes.iter().enumerate() {
        let class_idx = type_idx(&class.name);
        let _ = class_pos;
        for method in &class.methods {
            let proto_idx = *proto_index
                .get(&(method.ret.clone(), method.params.clone()))
                .expect("proto harus terdaftar");
            entries.push(MethodEntry {
                class_idx,
                proto_idx,
                name_idx: string_idx(&method.name),
                spec: method,
                method_idx: 0,
            });
        }
    }
    entries.sort_by_key(|e| (e.class_idx, e.name_idx, e.proto_idx));
    for (idx, entry) in entries.iter_mut().enumerate() {
        entry.method_idx = idx as u32;
    }

    // field_ids harus terurut (class_idx, name_idx, type_idx).
    struct FieldEntry<'a> {
        class_idx: u32,
        type_idx: u32,
        name_idx: u32,
        spec: &'a FieldSpec,
        field_idx: u32,
    }

    let mut field_entries: Vec<FieldEntry> = Vec::new();
    for class in classes {
        let class_idx = type_idx(&class.name);
        for field in &class.fields {
            field_entries.push(FieldEntry {
                class_idx,
                type_idx: type_idx(&field.field_type),
                name_idx: string_idx(&field.name),
                spec: field,
                field_idx: 0,
            });
        }
    }
    field_entries.sort_by_key(|e| (e.class_idx, e.name_idx, e.type_idx));
    for (idx, entry) in field_entries.iter_mut().enumerate() {
        entry.field_idx = idx as u32;
    }

    let string_ids_off = HEADER_SIZE;
    let type_ids_off = string_ids_off + 4 * strings.order.len() as u32;
    let proto_ids_off = type_ids_off + 4 * types.order.len() as u32;
    let field_ids_off = proto_ids_off + 12 * protos.len() as u32;
    let method_ids_off = field_ids_off + 8 * field_entries.len() as u32;
    let class_defs_off = method_ids_off + 8 * entries.len() as u32;
    let data_off = class_defs_off + 32 * classes.len() as u32;

    let mut data: Vec<u8> = Vec::new();
    let base = data_off;

    let mut string_offsets = Vec::with_capacity(strings.order.len());
    for value in &strings.order {
        string_offsets.push(base + data.len() as u32);
        push_string_data(&mut data, value);
    }

    let mut proto_params_off = vec![0u32; protos.len()];
    for (idx, proto) in protos.iter().enumerate() {
        if proto.2.is_empty() {
            continue;
        }
        align4(&mut data);
        proto_params_off[idx] = base + data.len() as u32;
        push_u32(&mut data, proto.2.len() as u32);
        for param in &proto.2 {
            // `type_item` pada type_list berukuran 2 byte (ushort).
            push_u16(&mut data, type_idx(param) as u16);
        }
    }

    // Code item ditulis sebelum class_data agar offset-nya sudah diketahui
    // saat encoding class_data.
    let mut code_offsets = vec![0u32; entries.len()];
    for entry in &entries {
        if entry.spec.insns.is_empty() {
            continue;
        }
        align4(&mut data);
        code_offsets[entry.method_idx as usize] = base + data.len() as u32;
        push_u16(&mut data, entry.spec.registers);
        push_u16(&mut data, entry.spec.ins);
        push_u16(&mut data, entry.spec.outs);
        push_u16(&mut data, entry.spec.tries.len() as u16);
        push_u32(&mut data, 0);
        push_u32(&mut data, entry.spec.insns.len() as u32);
        for insn in &entry.spec.insns {
            push_u16(&mut data, *insn);
        }
        if !entry.spec.insns.len().is_multiple_of(2) {
            push_u16(&mut data, 0);
        }

        if !entry.spec.tries.is_empty() {
            let mut handler_bytes: Vec<u8> = Vec::new();
            push_uleb128(&mut handler_bytes, entry.spec.handlers.len() as u32);
            let mut handler_offsets = Vec::with_capacity(entry.spec.handlers.len());
            for handler in &entry.spec.handlers {
                handler_offsets.push(handler_bytes.len() as u16);
                let size = handler.pairs.len() as i32;
                let encoded = if handler.catch_all.is_some() {
                    -size
                } else {
                    size
                };
                push_sleb128(&mut handler_bytes, encoded);
                for (descriptor, address) in &handler.pairs {
                    push_uleb128(&mut handler_bytes, type_idx(descriptor));
                    push_uleb128(&mut handler_bytes, *address);
                }
                if let Some(address) = handler.catch_all {
                    push_uleb128(&mut handler_bytes, address);
                }
            }
            for item in &entry.spec.tries {
                let offset = handler_offsets
                    .get(item.handler)
                    .copied()
                    .unwrap_or_else(|| panic!("handler {} tidak ada di fixture", item.handler));
                push_u32(&mut data, item.start);
                push_u16(&mut data, item.count);
                push_u16(&mut data, offset);
            }
            data.extend_from_slice(&handler_bytes);
        }
    }

    let mut class_data_offsets = vec![0u32; classes.len()];
    for (class_pos, class) in classes.iter().enumerate() {
        if class.methods.is_empty() && class.fields.is_empty() {
            continue;
        }
        align4(&mut data);
        class_data_offsets[class_pos] = base + data.len() as u32;

        let mut direct: Vec<&MethodEntry> = entries
            .iter()
            .filter(|e| e.class_idx == type_idx(&class.name) && e.spec.is_direct())
            .collect();
        let mut virtual_methods: Vec<&MethodEntry> = entries
            .iter()
            .filter(|e| e.class_idx == type_idx(&class.name) && !e.spec.is_direct())
            .collect();
        direct.sort_by_key(|e| e.method_idx);
        virtual_methods.sort_by_key(|e| e.method_idx);

        let mut static_fields: Vec<&FieldEntry> = field_entries
            .iter()
            .filter(|e| e.class_idx == type_idx(&class.name) && e.spec.is_static)
            .collect();
        let mut instance_fields: Vec<&FieldEntry> = field_entries
            .iter()
            .filter(|e| e.class_idx == type_idx(&class.name) && !e.spec.is_static)
            .collect();
        static_fields.sort_by_key(|e| e.field_idx);
        instance_fields.sort_by_key(|e| e.field_idx);

        push_uleb128(&mut data, static_fields.len() as u32);
        push_uleb128(&mut data, instance_fields.len() as u32);
        push_uleb128(&mut data, direct.len() as u32);
        push_uleb128(&mut data, virtual_methods.len() as u32);

        for group in [static_fields, instance_fields] {
            let mut last = 0u32;
            for entry in group {
                push_uleb128(&mut data, entry.field_idx - last);
                last = entry.field_idx;
                push_uleb128(&mut data, entry.spec.access_flags);
            }
        }

        for group in [direct, virtual_methods] {
            let mut last = 0u32;
            for entry in group {
                push_uleb128(&mut data, entry.method_idx - last);
                last = entry.method_idx;
                push_uleb128(&mut data, entry.spec.access_flags);
                push_uleb128(&mut data, code_offsets[entry.method_idx as usize]);
            }
        }
    }

    let file_size = data_off + data.len() as u32;

    let mut out = Vec::with_capacity(file_size as usize);
    out.extend_from_slice(b"dex\n035\0");
    push_u32(&mut out, 0);
    out.extend_from_slice(&[0u8; 20]);
    push_u32(&mut out, file_size);
    push_u32(&mut out, HEADER_SIZE);
    push_u32(&mut out, 0x1234_5678);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, strings.order.len() as u32);
    push_u32(&mut out, string_ids_off);
    push_u32(&mut out, types.order.len() as u32);
    push_u32(&mut out, type_ids_off);
    push_u32(&mut out, protos.len() as u32);
    push_u32(&mut out, proto_ids_off);
    push_u32(&mut out, field_entries.len() as u32);
    push_u32(&mut out, field_ids_off);
    push_u32(&mut out, entries.len() as u32);
    push_u32(&mut out, method_ids_off);
    push_u32(&mut out, classes.len() as u32);
    push_u32(&mut out, class_defs_off);
    push_u32(&mut out, data.len() as u32);
    push_u32(&mut out, data_off);

    for offset in &string_offsets {
        push_u32(&mut out, *offset);
    }
    for name in &types.order {
        push_u32(&mut out, string_idx(name));
    }
    for (idx, proto) in protos.iter().enumerate() {
        push_u32(&mut out, string_idx(&proto.0));
        push_u32(&mut out, type_idx(&proto.1));
        push_u32(&mut out, proto_params_off[idx]);
    }
    for entry in &field_entries {
        push_u16(&mut out, entry.class_idx as u16);
        push_u16(&mut out, entry.type_idx as u16);
        push_u32(&mut out, entry.name_idx);
    }
    for entry in &entries {
        push_u16(&mut out, entry.class_idx as u16);
        push_u16(&mut out, entry.proto_idx as u16);
        push_u32(&mut out, entry.name_idx);
    }
    for (class_pos, class) in classes.iter().enumerate() {
        push_u32(&mut out, type_idx(&class.name));
        push_u32(&mut out, class.access_flags);
        push_u32(
            &mut out,
            class
                .superclass
                .as_deref()
                .map(type_idx)
                .unwrap_or(NO_INDEX),
        );
        push_u32(&mut out, 0);
        push_u32(&mut out, NO_INDEX);
        push_u32(&mut out, 0);
        push_u32(&mut out, class_data_offsets[class_pos]);
        push_u32(&mut out, 0);
    }

    out.extend_from_slice(&data);
    debug_assert_eq!(out.len() as u32, file_size);

    let method_indices = entries
        .iter()
        .map(|entry| {
            (
                (
                    types.order[entry.class_idx as usize].clone(),
                    strings.order[entry.name_idx as usize].clone(),
                ),
                entry.method_idx as u16,
            )
        })
        .collect();

    let field_indices = field_entries
        .iter()
        .map(|entry| {
            (
                (
                    types.order[entry.class_idx as usize].clone(),
                    strings.order[entry.name_idx as usize].clone(),
                ),
                entry.field_idx as u16,
            )
        })
        .collect();

    let string_indices = strings
        .order
        .iter()
        .enumerate()
        .map(|(index, text)| (text.clone(), index as u16))
        .collect();

    let type_indices = types
        .order
        .iter()
        .enumerate()
        .map(|(index, descriptor)| (descriptor.clone(), index as u16))
        .collect();

    DexBuild {
        bytes: out,
        method_indices,
        field_indices,
        string_indices,
        type_indices,
    }
}

/// Helper encoding instruksi DEX yang dipakai tes.
pub mod insn {
    pub fn nop() -> Vec<u16> {
        vec![0x00]
    }

    /// move vA, vB (format 12x, register 4-bit).
    pub fn move_(dst: u8, src: u8) -> Vec<u16> {
        vec![0x01 | ((dst as u16) << 8) | ((src as u16) << 12)]
    }

    /// move/from16 vAA, vBBBB (format 22x).
    pub fn move_from16(dst: u8, src: u16) -> Vec<u16> {
        vec![0x02 | ((dst as u16) << 8), src]
    }

    /// move-result vAA (format 11x).
    pub fn move_result(dst: u8) -> Vec<u16> {
        vec![0x0a | ((dst as u16) << 8)]
    }

    pub fn return_void() -> Vec<u16> {
        vec![0x0e]
    }

    pub fn return_(reg: u8) -> Vec<u16> {
        vec![0x0f | ((reg as u16) << 8)]
    }

    /// return-object vAA (format 11x).
    pub fn return_object(reg: u8) -> Vec<u16> {
        vec![0x11 | ((reg as u16) << 8)]
    }

    /// const/4 vA, #+B (format 11n).
    pub fn const_4(reg: u8, literal: i8) -> Vec<u16> {
        vec![0x12 | ((reg as u16) << 8) | (((literal as u16) & 0xF) << 12)]
    }

    /// const-string vAA, string@BBBB (format 21c).
    pub fn const_string(reg: u8, string_index: u16) -> Vec<u16> {
        vec![0x1a | ((reg as u16) << 8), string_index]
    }

    /// Binop format 12x: vA op= vB (vA menjadi tujuan dan operand kiri).
    pub fn binop_2addr(opcode: u16, dst: u8, src: u8) -> Vec<u16> {
        vec![opcode | ((dst as u16) << 8) | ((src as u16) << 12)]
    }

    /// const-string/jumbo vAA, string@BBBBBBBB (format 31c).
    pub fn const_string_jumbo(reg: u8, string_index: u32) -> Vec<u16> {
        vec![
            0x1b | ((reg as u16) << 8),
            (string_index & 0xFFFF) as u16,
            (string_index >> 16) as u16,
        ]
    }

    /// new-instance vAA, type@BBBB (format 21c).
    pub fn new_instance(reg: u8, type_index: u16) -> Vec<u16> {
        vec![0x22 | ((reg as u16) << 8), type_index]
    }

    /// const-class vAA, type@BBBB (format 21c).
    pub fn const_class(reg: u8, type_index: u16) -> Vec<u16> {
        vec![0x1c | ((reg as u16) << 8), type_index]
    }

    /// monitor-enter vAA (format 11x).
    pub fn monitor_enter(reg: u8) -> Vec<u16> {
        vec![0x1d | ((reg as u16) << 8)]
    }

    /// monitor-exit vAA (format 11x).
    pub fn monitor_exit(reg: u8) -> Vec<u16> {
        vec![0x1e | ((reg as u16) << 8)]
    }

    /// check-cast vAA, type@BBBB (format 21c).
    pub fn check_cast(reg: u8, type_index: u16) -> Vec<u16> {
        vec![0x1f | ((reg as u16) << 8), type_index]
    }

    /// instance-of vA, vB, type@CCCC (format 22c).
    pub fn instance_of(dst: u8, object: u8, type_index: u16) -> Vec<u16> {
        vec![
            0x20 | ((dst as u16) << 8) | ((object as u16) << 12),
            type_index,
        ]
    }

    /// filled-new-array {vC..}, type@BBBB (format 35c).
    pub fn filled_new_array(type_index: u16, args: &[u8]) -> Vec<u16> {
        invoke(0x24, type_index, args)
    }

    /// fill-array-data vAA, +BBBBBBBB (format 31t, offset dalam code unit).
    pub fn fill_array_data(reg: u8, offset: i32) -> Vec<u16> {
        vec![
            0x26 | ((reg as u16) << 8),
            (offset & 0xFFFF) as u16,
            ((offset >> 16) & 0xFFFF) as u16,
        ]
    }

    /// invoke-interface {vC..}, method@BBBB (format 35c).
    pub fn invoke_interface(method_idx: u16, args: &[u8]) -> Vec<u16> {
        invoke(0x72, method_idx, args)
    }

    /// Payload `fill-array-data` untuk data mentah little-endian.
    pub fn array_data_payload(width: u16, data: &[u8]) -> Vec<u16> {
        assert!(matches!(width, 1 | 2 | 4 | 8), "lebar elemen tidak valid");
        assert_eq!(
            data.len() % width as usize,
            0,
            "panjang data bukan kelipatan lebar elemen"
        );
        let size = (data.len() / width as usize) as u32;
        let mut units = vec![
            0x0300,
            width,
            (size & 0xFFFF) as u16,
            ((size >> 16) & 0xFFFF) as u16,
        ];
        let mut bytes = data.to_vec();
        if !bytes.len().is_multiple_of(2) {
            bytes.push(0);
        }
        for chunk in bytes.chunks(2) {
            units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        units
    }

    /// Payload `fill-array-data` untuk array int.
    pub fn array_data_i32(values: &[i32]) -> Vec<u16> {
        let mut bytes = Vec::with_capacity(values.len() * 4);
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        array_data_payload(4, &bytes)
    }

    /// Instruksi aritmetika int dengan literal 16-bit (format 22s).
    pub fn binop_lit16(opcode: u16, dst: u8, src: u8, literal: i16) -> Vec<u16> {
        vec![
            opcode | ((dst as u16) << 8),
            ((src as u16) << 12) | (literal as u16),
        ]
    }

    /// Instruksi aritmetika int dengan literal 8-bit (format 22b).
    pub fn binop_lit8(opcode: u16, dst: u8, src: u8, literal: i8) -> Vec<u16> {
        vec![
            opcode | ((dst as u16) << 8),
            ((literal as u8 as u16) << 8) | src as u16,
        ]
    }

    /// packed-switch vAA, +BBBBBBBB (format 31t).
    pub fn packed_switch(reg: u8, offset: i32) -> Vec<u16> {
        vec![
            0x2b | ((reg as u16) << 8),
            (offset & 0xFFFF) as u16,
            ((offset >> 16) & 0xFFFF) as u16,
        ]
    }

    /// sparse-switch vAA, +BBBBBBBB (format 31t).
    pub fn sparse_switch(reg: u8, offset: i32) -> Vec<u16> {
        vec![
            0x2c | ((reg as u16) << 8),
            (offset & 0xFFFF) as u16,
            ((offset >> 16) & 0xFFFF) as u16,
        ]
    }

    /// Payload packed-switch: ident 0x0100, ukuran, kunci pertama, target.
    pub fn packed_switch_payload(first_key: i32, targets: &[i32]) -> Vec<u16> {
        let mut units = vec![0x0100, targets.len() as u16];
        units.extend(i32_units(first_key));
        for target in targets {
            units.extend(i32_units(*target));
        }
        units
    }

    /// Payload sparse-switch: ident 0x0200, ukuran, daftar kunci, daftar target.
    pub fn sparse_switch_payload(keys: &[i32], targets: &[i32]) -> Vec<u16> {
        assert_eq!(
            keys.len(),
            targets.len(),
            "kunci dan target harus sama panjang"
        );
        let mut units = vec![0x0200, keys.len() as u16];
        for key in keys {
            units.extend(i32_units(*key));
        }
        for target in targets {
            units.extend(i32_units(*target));
        }
        units
    }

    fn i32_units(value: i32) -> [u16; 2] {
        [(value & 0xFFFF) as u16, ((value >> 16) & 0xFFFF) as u16]
    }

    /// array-length vA, vB (format 12x).
    pub fn array_length(dst: u8, array: u8) -> Vec<u16> {
        vec![0x21 | ((dst as u16) << 8) | ((array as u16) << 12)]
    }

    /// new-array vA, vB, type@CCCC (format 22c).
    pub fn new_array(dst: u8, size_reg: u8, type_index: u16) -> Vec<u16> {
        vec![
            0x23 | ((dst as u16) << 8) | ((size_reg as u16) << 12),
            type_index,
        ]
    }

    /// aget vAA, vBB, vCC (format 23x).
    pub fn aget(dst: u8, array: u8, index: u8) -> Vec<u16> {
        vec![
            0x44 | ((dst as u16) << 8),
            (array as u16) | ((index as u16) << 8),
        ]
    }

    /// aput vAA, vBB, vCC (format 23x).
    pub fn aput(src: u8, array: u8, index: u8) -> Vec<u16> {
        vec![
            0x4b | ((src as u16) << 8),
            (array as u16) | ((index as u16) << 8),
        ]
    }

    /// iget vA, vB, field@CCCC (format 22c).
    pub fn iget(dst: u8, object: u8, field_index: u16) -> Vec<u16> {
        vec![
            0x52 | ((dst as u16) << 8) | ((object as u16) << 12),
            field_index,
        ]
    }

    /// iget-object vA, vB, field@CCCC (format 22c).
    pub fn iget_object(dst: u8, object: u8, field_index: u16) -> Vec<u16> {
        vec![
            0x54 | ((dst as u16) << 8) | ((object as u16) << 12),
            field_index,
        ]
    }

    /// iput vA, vB, field@CCCC (format 22c).
    pub fn iput(src: u8, object: u8, field_index: u16) -> Vec<u16> {
        vec![
            0x59 | ((src as u16) << 8) | ((object as u16) << 12),
            field_index,
        ]
    }

    /// sget vAA, field@BBBB (format 21c).
    pub fn sget(dst: u8, field_index: u16) -> Vec<u16> {
        vec![0x60 | ((dst as u16) << 8), field_index]
    }

    /// sput vAA, field@BBBB (format 21c).
    pub fn sput(src: u8, field_index: u16) -> Vec<u16> {
        vec![0x67 | ((src as u16) << 8), field_index]
    }

    /// throw vAA (format 11x).
    pub fn throw_(reg: u8) -> Vec<u16> {
        vec![0x27 | ((reg as u16) << 8)]
    }

    /// move-exception vAA (format 11x).
    pub fn move_exception(dst: u8) -> Vec<u16> {
        vec![0x0d | ((dst as u16) << 8)]
    }

    /// const/16 vAA, #+BBBB (format 21s).
    pub fn const_16(reg: u8, literal: i16) -> Vec<u16> {
        vec![0x13 | ((reg as u16) << 8), literal as u16]
    }

    /// const vAA, #+BBBBBBBB (format 31i).
    pub fn const_(reg: u8, literal: i32) -> Vec<u16> {
        vec![
            0x14 | ((reg as u16) << 8),
            (literal & 0xFFFF) as u16,
            ((literal >> 16) & 0xFFFF) as u16,
        ]
    }

    /// const/high16 vAA, #+BBBB0000 (format 21h).
    pub fn const_high16(reg: u8, literal: i16) -> Vec<u16> {
        vec![0x15 | ((reg as u16) << 8), literal as u16]
    }

    /// const-wide/16 vAA, #+BBBB (format 21s).
    pub fn const_wide_16(reg: u8, literal: i16) -> Vec<u16> {
        vec![0x16 | ((reg as u16) << 8), literal as u16]
    }

    /// const-wide/32 vAA, #+BBBBBBBB (format 31i).
    pub fn const_wide_32(reg: u8, literal: i32) -> Vec<u16> {
        vec![
            0x17 | ((reg as u16) << 8),
            (literal & 0xFFFF) as u16,
            ((literal >> 16) & 0xFFFF) as u16,
        ]
    }

    /// const-wide vAA, #+BBBBBBBBBBBBBBBB (format 51l).
    pub fn const_wide(reg: u8, literal: i64) -> Vec<u16> {
        let bits = literal as u64;
        vec![
            0x18 | ((reg as u16) << 8),
            (bits & 0xFFFF) as u16,
            ((bits >> 16) & 0xFFFF) as u16,
            ((bits >> 32) & 0xFFFF) as u16,
            ((bits >> 48) & 0xFFFF) as u16,
        ]
    }

    /// return-wide vAA (format 11x).
    pub fn return_wide(reg: u8) -> Vec<u16> {
        vec![0x10 | ((reg as u16) << 8)]
    }

    /// move-object vA, vB (format 12x).
    pub fn move_object(dst: u8, src: u8) -> Vec<u16> {
        vec![0x07 | ((dst as u16) << 8) | ((src as u16) << 12)]
    }

    /// move-object/from16 vAA, vBBBB (format 22x).
    pub fn move_object_from16(dst: u8, src: u16) -> Vec<u16> {
        vec![0x08 | ((dst as u16) << 8), src]
    }

    /// move-object/16 vAAAA, vBBBB (format 32x).
    pub fn move_object_16(dst: u16, src: u16) -> Vec<u16> {
        vec![0x09, dst, src]
    }

    /// move-wide vA, vB (format 12x).
    pub fn move_wide(dst: u8, src: u8) -> Vec<u16> {
        vec![0x04 | ((dst as u16) << 8) | ((src as u16) << 12)]
    }

    /// Instruksi 23x (dst, a, b): aritmetika long/float/double dan cmp.
    pub fn ternary(opcode: u16, dst: u8, a: u8, b: u8) -> Vec<u16> {
        vec![opcode | ((dst as u16) << 8), (a as u16) | ((b as u16) << 8)]
    }

    /// Instruksi 12x (dst, src): konversi dan negasi.
    pub fn unary(opcode: u16, dst: u8, src: u8) -> Vec<u16> {
        vec![opcode | ((dst as u16) << 8) | ((src as u16) << 12)]
    }

    pub fn add_long(dst: u8, a: u8, b: u8) -> Vec<u16> {
        ternary(0x9b, dst, a, b)
    }

    pub fn div_long(dst: u8, a: u8, b: u8) -> Vec<u16> {
        ternary(0x9e, dst, a, b)
    }

    pub fn add_float(dst: u8, a: u8, b: u8) -> Vec<u16> {
        ternary(0xa6, dst, a, b)
    }

    pub fn add_double(dst: u8, a: u8, b: u8) -> Vec<u16> {
        ternary(0xab, dst, a, b)
    }

    pub fn cmp_long(dst: u8, a: u8, b: u8) -> Vec<u16> {
        ternary(0x31, dst, a, b)
    }

    pub fn int_to_long(dst: u8, src: u8) -> Vec<u16> {
        unary(0x81, dst, src)
    }

    pub fn long_to_int(dst: u8, src: u8) -> Vec<u16> {
        unary(0x84, dst, src)
    }

    pub fn int_to_float(dst: u8, src: u8) -> Vec<u16> {
        unary(0x82, dst, src)
    }

    pub fn float_to_int(dst: u8, src: u8) -> Vec<u16> {
        unary(0x87, dst, src)
    }

    /// goto +AA (format 10t, offset dalam code unit).
    pub fn goto(offset: i8) -> Vec<u16> {
        vec![0x28 | (((offset as u16) & 0xFF) << 8)]
    }

    /// if-eq vA, vB, +CCCC (format 22t).
    pub fn if_eq(a: u8, b: u8, offset: i16) -> Vec<u16> {
        vec![0x32 | ((a as u16) << 8) | ((b as u16) << 12), offset as u16]
    }

    /// if-ne vA, vB, +CCCC (format 22t).
    pub fn if_ne(a: u8, b: u8, offset: i16) -> Vec<u16> {
        vec![0x33 | ((a as u16) << 8) | ((b as u16) << 12), offset as u16]
    }

    /// if-lt vA, vB, +CCCC (format 22t).
    pub fn if_lt(a: u8, b: u8, offset: i16) -> Vec<u16> {
        vec![0x34 | ((a as u16) << 8) | ((b as u16) << 12), offset as u16]
    }

    /// if-ge vA, vB, +CCCC (format 22t).
    pub fn if_ge(a: u8, b: u8, offset: i16) -> Vec<u16> {
        vec![0x35 | ((a as u16) << 8) | ((b as u16) << 12), offset as u16]
    }

    /// if-eqz vAA, +BBBB (format 21t).
    pub fn if_eqz(reg: u8, offset: i16) -> Vec<u16> {
        vec![0x38 | ((reg as u16) << 8), offset as u16]
    }

    fn binop(opcode: u16, dst: u8, a: u8, b: u8) -> Vec<u16> {
        // 23x: vAA di byte tinggi unit pertama, vBB/vCC di unit kedua.
        vec![opcode | ((dst as u16) << 8), (a as u16) | ((b as u16) << 8)]
    }

    pub fn add_int(dst: u8, a: u8, b: u8) -> Vec<u16> {
        binop(0x90, dst, a, b)
    }

    pub fn sub_int(dst: u8, a: u8, b: u8) -> Vec<u16> {
        binop(0x91, dst, a, b)
    }

    pub fn mul_int(dst: u8, a: u8, b: u8) -> Vec<u16> {
        binop(0x92, dst, a, b)
    }

    pub fn div_int(dst: u8, a: u8, b: u8) -> Vec<u16> {
        binop(0x93, dst, a, b)
    }

    pub fn rem_int(dst: u8, a: u8, b: u8) -> Vec<u16> {
        binop(0x94, dst, a, b)
    }

    /// invoke-static {vC}, method@BBBB (format 35c).
    pub fn invoke_static(method_idx: u16, args: &[u8]) -> Vec<u16> {
        invoke(0x71, method_idx, args)
    }

    /// invoke-static/range {vCCCC .. vNNNN}, method@BBBB (format 3rc).
    pub fn invoke_static_range(method_idx: u16, first: u16, count: u8) -> Vec<u16> {
        vec![0x77 | ((count as u16) << 8), method_idx, first]
    }

    /// invoke-virtual/range {vCCCC .. vNNNN}, method@BBBB (format 3rc).
    pub fn invoke_virtual_range(method_idx: u16, first: u16, count: u8) -> Vec<u16> {
        vec![0x73 | ((count as u16) << 8), method_idx, first]
    }

    /// invoke-direct {vC}, method@BBBB (format 35c).
    pub fn invoke_direct(method_idx: u16, args: &[u8]) -> Vec<u16> {
        invoke(0x70, method_idx, args)
    }

    /// invoke-virtual {vC}, method@BBBB (format 35c).
    pub fn invoke_virtual(method_idx: u16, args: &[u8]) -> Vec<u16> {
        invoke(0x6e, method_idx, args)
    }

    fn invoke(opcode: u16, method_idx: u16, args: &[u8]) -> Vec<u16> {
        assert!(args.len() <= 5, "format 35c maksimal 5 argumen");
        let count = args.len() as u16;
        let g = if count == 5 { args[4] as u16 } else { 0 };
        let mut unit2 = 0u16;
        if count > 0 {
            unit2 |= args[0] as u16;
        }
        if count > 1 {
            unit2 |= (args[1] as u16) << 4;
        }
        if count > 2 {
            unit2 |= (args[2] as u16) << 8;
        }
        if count > 3 {
            unit2 |= (args[3] as u16) << 12;
        }
        vec![
            opcode | ((count & 0xF) << 12) | ((g & 0xF) << 8),
            method_idx,
            unit2,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_dex_menghasilkan_header_dan_tabel_konsisten() {
        let class = ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new("jawab", ACC_PUBLIC | ACC_STATIC, &[], "I")
                .registers(1)
                .body([insn::const_16(0, 42), insn::return_(0)].concat()),
        );

        let dex = build_dex(&[class]);
        assert_eq!(&dex[..8], b"dex\n035\0");
        let file_size = u32::from_le_bytes([dex[32], dex[33], dex[34], dex[35]]) as usize;
        assert_eq!(file_size, dex.len());
        assert_eq!(u32::from_le_bytes([dex[88], dex[89], dex[90], dex[91]]), 1);
        assert_eq!(u32::from_le_bytes([dex[96], dex[97], dex[98], dex[99]]), 1);
    }

    #[test]
    fn insn_helper_menghasilkan_format_yang_benar() {
        assert_eq!(insn::const_4(1, -1), vec![0x12 | (1 << 8) | (0xF << 12)]);
        assert_eq!(insn::const_16(2, 42), vec![0x13 | (2 << 8), 42]);
        assert_eq!(insn::return_(3), vec![0x0f | (3 << 8)]);
        assert_eq!(insn::if_eq(1, 2, 3), vec![0x32 | (1 << 8) | (2 << 12), 3]);
    }
}
