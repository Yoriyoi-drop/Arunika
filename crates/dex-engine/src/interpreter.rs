//! Interpreter DEX subset untuk fase 1.
//!
//! Cakupan: konstanta (termasuk string), move, aritmetika int, percabangan,
//! return, new-instance, throw/move-exception dengan try/catch, serta invoke
//! direct/static/virtual. Pemanggilan diarahkan ke method DEX lain bila ada,
//! atau ke NativeResolver untuk API yang disediakan runtime.
//!
//! Eksepsi ditangani dengan rantai superclass dari `class_defs`, sehingga
//! `catch` pada tipe induk ikut menangkap subclass-nya. Interface belum
//! dicocokkan: hanya tipe persis dan superclass-nya.
//!
//! Belum ada: JNI dan GC. Field, array, long/float/double, dan finally
//! (via catch-all + rethrow) sudah didukung.
//! Instruksi di luar cakupan dilaporkan sebagai error eksplisit.

use std::collections::HashMap;

use crate::value::{Heap, Object, Value};
use crate::{ClassState, CodeItem, DexEngine, DexError, DexFile, ExecutionOutcome, NativeResolver};

/// Batas jumlah elemen array agar `new-array` tidak mengalokasikan memori
/// raksasa dari ukuran yang dikendalikan bytecode.
pub const MAX_ARRAY_LEN: u32 = 1 << 20;

/// Hasil satu frame: nilai kembali, atau eksepsi yang merambat ke pemanggil.
///
/// Eksepsi sengaja bukan `DexError` agar pemanggil dapat mencocokkannya
/// dengan handler try/catch miliknya sendiri.
enum FrameExit {
    Return(Option<Value>),
    Throw(Value),
}

/// Hasil satu instruksi.
enum Step {
    /// Lanjut pada pc baru.
    Goto(usize),
    /// Method selesai dengan nilai kembali (atau tanpa nilai).
    Return(Option<Value>),
    /// Instruksi melempar nilai; frame mencari handler yang cocok.
    Throw(Value),
    /// Panggil method lain; frame yang menjalankannya.
    Invoke { method_index: u32, args: Vec<Value> },
}

/// Batas langkah eksekusi per pemanggilan agar bytecode tidak menggantung.
pub const MAX_STEPS: u64 = 1_000_000;

/// Batas kedalaman pemanggilan method.
pub const MAX_CALL_DEPTH: usize = 64;

/// Batas penelusuran rantai superclass saat mencocokkan tipe eksepsi.
const MAX_TYPE_DEPTH: usize = 64;

impl DexFile {
    /// Indeks method pada method_ids untuk pasangan class/method.
    pub fn method_index(&self, class_descriptor: &str, method_name: &str) -> Option<usize> {
        self.method_ids
            .iter()
            .position(|m| m.class == class_descriptor && m.name == method_name)
    }

    /// Tipe parameter method, dipakai untuk melewati word kedua nilai wide.
    pub fn method_parameter_types(&self, method_index: u32) -> Option<&[String]> {
        let method = self.method_ids.get(method_index as usize)?;
        let proto = self.proto_ids.get(method.proto_idx as usize)?;
        Some(&proto.parameter_types)
    }

    /// Signature method dalam format DEX, misalnya `(ILjava/lang/String;)V`.
    pub fn method_signature(&self, method_index: u32) -> String {
        let Some(method) = self.method_ids.get(method_index as usize) else {
            return String::new();
        };
        let Some(proto) = self.proto_ids.get(method.proto_idx as usize) else {
            return String::new();
        };
        let mut signature = String::from("(");
        for parameter in &proto.parameter_types {
            signature.push_str(parameter);
        }
        signature.push(')');
        signature.push_str(&proto.return_type);
        signature
    }

    /// Indeks method berdasarkan class, nama, dan tipe parameter.
    ///
    /// Reflection (`Class.getMethod`) hanya menyebut tipe parameter, bukan tipe
    /// kembali, jadi pencocokan tidak memakai signature penuh.
    pub fn find_method_by_parameters(
        &self,
        class_descriptor: &str,
        method_name: &str,
        parameters: &[String],
    ) -> Option<u32> {
        self.method_ids
            .iter()
            .enumerate()
            .find_map(|(index, method)| {
                if method.class != class_descriptor || method.name != method_name {
                    return None;
                }
                let types = self.method_parameter_types(index as u32)?;
                (types == parameters).then_some(index as u32)
            })
    }

    /// Badan method berdasarkan indeks method_ids.
    pub fn code_item_for_method(&self, method_index: u32) -> Option<&CodeItem> {
        let index = *self.code_item_by_method.get(&method_index)?;
        self.code_items.get(index)
    }
}

impl DexEngine {
    /// Pasang penyedia method native (framework API).
    pub fn set_native_resolver(&mut self, resolver: Box<dyn NativeResolver>) {
        self.native = Some(resolver);
    }

    /// Buat instance class lalu jalankan `<init>` bila tersedia.
    ///
    /// Instance dialokasikan lebih dulu agar `<init>` menerima `this` yang
    /// sama dengan objek hasil. Class tanpa `<init>` tetap menghasilkan objek.
    /// `<init>` tanpa kode diteruskan ke resolver; bila resolver juga tidak
    /// menyediakannya, `NoCode` dilaporkan agar kekurangan API terlihat.
    pub fn instantiate_class(&mut self, class_descriptor: &str) -> Result<Value, DexError> {
        // Instance pertama memicu initializer class, sama seperti new-instance.
        let mut steps = 0u64;
        ensure_class_initialized(
            &self.dex,
            &mut self.heap,
            &mut self.statics,
            &mut self.classes,
            &mut self.class_state,
            self.native.as_deref(),
            class_descriptor,
            0,
            &mut steps,
        )?;
        self.last_steps = steps;
        let object = self.heap.alloc(Object::Instance {
            type_name: class_descriptor.to_owned(),
            fields: HashMap::new(),
        });

        let Some(index) = self.dex.method_index(class_descriptor, "<init>") else {
            return Ok(object);
        };
        let index = index as u32;

        if self.dex.code_item_for_method(index).is_some() {
            self.execute_method_values(class_descriptor, "<init>", &[object])?;
            return Ok(object);
        }

        let signature = self.dex.method_signature(index);
        let native = self.native.as_deref();
        let resolved = native.and_then(|resolver| {
            resolver.invoke_described(
                &mut self.heap,
                class_descriptor,
                "<init>",
                &signature,
                &[object],
            )
        });
        match resolved {
            Some(Ok(_)) => Ok(object),
            Some(Err(message)) => Err(DexError::NativeFailure {
                class: class_descriptor.to_owned(),
                method: "<init>".to_owned(),
                message,
            }),
            None => Err(DexError::NoCode {
                class: class_descriptor.to_owned(),
                method: "<init>".to_owned(),
                signature: signature.clone(),
            }),
        }
    }

    /// Jalankan `<clinit>` class bila method itu ada dan memiliki kode.
    ///
    /// Mengembalikan `true` bila initializer dijalankan. Pemanggil yang
    /// membutuhkan state statis siap harus memanggil ini sebelum method sasaran.
    pub fn execute_class_initializer(&mut self, class_descriptor: &str) -> Result<bool, DexError> {
        let Some(index) = self.dex.method_index(class_descriptor, "<clinit>") else {
            return Ok(false);
        };
        if self.dex.code_item_for_method(index as u32).is_none() {
            return Ok(false);
        }
        self.execute_method_values(class_descriptor, "<clinit>", &[])?;
        self.class_state
            .initialized
            .insert(class_descriptor.to_string());
        Ok(true)
    }

    /// Jalankan method tanpa argumen.
    pub fn execute_method(
        &mut self,
        class_descriptor: &str,
        method_name: &str,
    ) -> Result<ExecutionOutcome, DexError> {
        self.execute_method_values(class_descriptor, method_name, &[])
    }

    /// Jalankan method dengan argumen int.
    pub fn execute_method_with_args(
        &mut self,
        class_descriptor: &str,
        method_name: &str,
        args: &[i64],
    ) -> Result<ExecutionOutcome, DexError> {
        let values: Vec<Value> = args.iter().map(|value| Value::Int(*value as i32)).collect();
        self.execute_method_values(class_descriptor, method_name, &values)
    }

    /// Jalankan method dengan nilai register parameter eksplisit.
    ///
    /// Nilai diletakkan pada register parameter terakhir sesuai konvensi DEX
    /// (parameter menempati register paling tinggi).
    pub fn execute_method_values(
        &mut self,
        class_descriptor: &str,
        method_name: &str,
        args: &[Value],
    ) -> Result<ExecutionOutcome, DexError> {
        let method_index = self
            .dex
            .method_index(class_descriptor, method_name)
            .ok_or_else(|| DexError::MethodNotFound {
                class: class_descriptor.to_owned(),
                method: method_name.to_owned(),
            })?;

        let code = self
            .dex
            .code_item_for_method(method_index as u32)
            .ok_or_else(|| DexError::NoCode {
                class: class_descriptor.to_owned(),
                method: method_name.to_owned(),
                signature: self.dex.method_signature(method_index as u32),
            })?
            .clone();

        let mut steps = 0u64;
        let exit = annotate_limit(
            run_frame(
                &self.dex,
                &mut self.heap,
                &mut self.statics,
                &mut self.classes,
                &mut self.class_state,
                self.native.as_deref(),
                &code,
                args,
                0,
                &mut steps,
            ),
            class_descriptor,
            method_name,
        )?;

        let return_value = match exit {
            FrameExit::Return(value) => value,
            FrameExit::Throw(value) => {
                return Err(DexError::UncaughtException {
                    type_name: thrown_type_name(&self.heap, value),
                });
            }
        };

        self.instruction_pointer = code.instructions_size;
        self.return_value = return_value;
        self.last_steps = steps;
        self.registers = vec![Value::Int(0); code.registers_size as usize];

        Ok(ExecutionOutcome {
            executed: true,
            reason: format!("eksekusi selesai dalam {steps} langkah"),
            next_instruction_pointer: code.instructions_size,
            return_value,
            steps,
        })
    }

    /// Jalankan tugas `java.util.Timer` yang sudah terdaftar.
    ///
    /// Runtime ini kooperatif: tidak ada jam dinding, thread penjadwal, atau
    /// pengulangan periodik. Setiap tugas yang dijadwalkan selama lifecycle
    /// dijalankan satu kali pada titik ini, berurutan sesuai pendaftaran, lalu
    /// dihapus dari timernya. Timer yang dibatalkan dilewati.
    pub fn run_due_timers(&mut self) -> Result<u64, DexError> {
        self.drain_tasks(&["Ljava/util/Timer;"], Some(0), "run")
    }

    /// Jalankan `Runnable` yang diposting ke `android.os.Handler`.
    ///
    /// `post` hanya mencatat antrean; engine menjalankan `run` setiap Runnable
    /// sekali pada titik ini, seperti pesan yang diproses setelah lifecycle.
    pub fn run_due_handlers(&mut self) -> Result<u64, DexError> {
        self.drain_tasks(&["Landroid/os/Handler;"], None, "run")
    }

    /// Jalankan tugas yang diserahkan ke executor.
    ///
    /// Executor di sini kooperatif: tugas diantrekan oleh `execute`/`submit`
    /// lalu dijalankan sekali pada titik ini, bukan oleh thread pekerja.
    pub fn run_due_executors(&mut self) -> Result<u64, DexError> {
        self.drain_tasks(
            &[
                "Ljava/util/concurrent/ExecutorService;",
                "Ljava/util/concurrent/ThreadPoolExecutor;",
            ],
            None,
            "run",
        )
    }

    /// Jalankan method tanpa argumen pada setiap tugas di field 1 milik
    /// instance `owner`, lalu kosongkan antreannya.
    ///
    /// `cancelled_field` menunjuk field penanda batal; bila bernilai bukan nol
    /// instance dilewati tanpa mengubah antreannya.
    fn drain_tasks(
        &mut self,
        owners: &[&str],
        cancelled_field: Option<u32>,
        task_method: &str,
    ) -> Result<u64, DexError> {
        let mut jobs: Vec<Value> = Vec::new();
        for index in 0..self.heap.len() as u32 {
            let Some(Object::Instance { type_name, fields }) = self.heap.get(index) else {
                continue;
            };
            if !owners.contains(&type_name.as_str()) {
                continue;
            }
            if let Some(field) = cancelled_field {
                let cancelled = fields
                    .get(&field)
                    .and_then(|value| value.as_int())
                    .unwrap_or(0)
                    != 0;
                if cancelled {
                    continue;
                }
            }
            let Some(tasks) = fields.get(&1).and_then(|value| value.as_ref()) else {
                continue;
            };
            if let Some(Object::List { elements, .. }) = self.heap.get_mut(tasks) {
                jobs.append(elements);
            }
        }

        let mut steps = 0u64;
        let mut executed = 0u64;
        for task in jobs {
            let Some(class) = task
                .as_ref()
                .and_then(|reference| self.heap.get(reference))
                .and_then(|object| match object {
                    Object::Instance { type_name, .. } => Some(type_name.clone()),
                    _ => None,
                })
            else {
                continue;
            };
            let method_index = self
                .dex
                .find_method_by_parameters(&class, task_method, &[])
                .or_else(|| self.dex.find_method_by_parameters(&class, "call", &[]));
            let Some(method_index) = method_index else {
                continue;
            };
            let outcome = call_method(
                &self.dex,
                &mut self.heap,
                &mut self.statics,
                &mut self.classes,
                &mut self.class_state,
                self.native.as_deref(),
                method_index,
                &[task],
                0,
                &mut steps,
            );
            match outcome {
                Ok(FrameExit::Return(_)) => executed += 1,
                Ok(FrameExit::Throw(value)) => {
                    self.last_steps = steps;
                    return Err(DexError::UncaughtException {
                        type_name: thrown_type_name(&self.heap, value),
                    });
                }
                // Langkah yang sudah terpakai tetap dilaporkan meski tugas gagal.
                Err(error) => {
                    self.last_steps = steps;
                    return Err(error);
                }
            }
        }

        self.last_steps = steps;
        Ok(executed)
    }
}

/// Descriptor tipe dari objek yang dilempar.
///
/// Referensi hanya dibuat oleh `Heap::alloc`, jadi referensi yang tidak
/// ditemukan menandakan model nilai yang rusak; kasus itu dilaporkan sebagai
/// `Throwable` generik alih-alih panik.
fn thrown_type_name(heap: &Heap, value: Value) -> String {
    value_descriptor(heap, value).unwrap_or_else(|| "Ljava/lang/Throwable;".to_owned())
}

/// Nilai awal field sesuai tipe DEX-nya.
///
/// Field `long`/`double` menempati dua register, jadi nilai awalnya harus
/// `Wide(0)`; memakai `Int(0)` membuat operasi berikutnya gagal karena
/// lebarnya tidak cocok.
fn zero_for_field_type(descriptor: &str) -> Value {
    match descriptor {
        "J" | "D" => Value::Wide(0),
        _ => Value::Int(0),
    }
}

/// Descriptor tipe dari nilai referensi; `None` untuk nilai non-referensi.
fn value_descriptor(heap: &Heap, value: Value) -> Option<String> {
    match value.as_ref().and_then(|index| heap.get(index)) {
        Some(Object::Instance { type_name, .. }) => Some(type_name.clone()),
        Some(Object::String(_)) => Some("Ljava/lang/String;".to_owned()),
        Some(Object::Array { element_type, .. }) => Some(format!("[{element_type}")),
        Some(Object::Class { .. }) => Some("Ljava/lang/Class;".to_owned()),
        Some(Object::Method { .. }) => Some("Ljava/lang/reflect/Method;".to_owned()),
        Some(Object::List { type_name, .. })
        | Some(Object::Boxed { type_name, .. })
        | Some(Object::Map { type_name, .. })
        | Some(Object::Stream { type_name, .. }) => Some(type_name.clone()),
        Some(Object::Bitmap { .. }) => Some("Landroid/graphics/Bitmap;".to_owned()),
        None => None,
    }
}

/// Apakah `thrown` dapat ditangkap oleh handler bertipe `catch_type`.
///
/// Pencocokan mengikuti rantai superclass pada `class_defs`. Interface belum
/// dipertimbangkan, sehingga `catch` pada interface tidak menangkap apa pun.
fn is_assignable_to(dex: &DexFile, thrown: &str, catch_type: &str) -> bool {
    // Semua referensi dapat diperlakukan sebagai Object.
    if catch_type == "Ljava/lang/Object;" {
        return true;
    }

    let mut current = Some(thrown);
    let mut guard = 0usize;
    while let Some(name) = current {
        if name == catch_type {
            return true;
        }
        guard += 1;
        if guard > MAX_TYPE_DEPTH {
            return false;
        }
        current = dex
            .class_defs
            .iter()
            .find(|def| def.class == name)
            .and_then(|def| def.superclass.as_deref())
            .or_else(|| builtin_superclass(name));
    }
    false
}

/// Hierarki kelas framework yang tidak ada di DEX aplikasi.
///
/// Eksepsi yang dilempar runtime (mis. FileNotFoundException) tidak punya
/// class_def, sehingga rantai superclass-nya harus disediakan di sini agar
/// catch pada IOException tetap menangkapnya.
fn builtin_superclass(name: &str) -> Option<&'static str> {
    Some(match name {
        "Ljava/io/FileNotFoundException;" => "Ljava/io/IOException;",
        "Ljava/io/IOException;" => "Ljava/lang/Exception;",
        "Ljava/lang/RuntimeException;" => "Ljava/lang/Exception;",
        "Ljava/lang/SecurityException;" => "Ljava/lang/RuntimeException;",
        "Ljava/lang/IllegalArgumentException;" => "Ljava/lang/RuntimeException;",
        "Ljava/lang/IllegalStateException;" => "Ljava/lang/RuntimeException;",
        "Ljava/lang/NullPointerException;" => "Ljava/lang/RuntimeException;",
        "Ljava/lang/ArithmeticException;" => "Ljava/lang/RuntimeException;",
        "Ljava/lang/ClassCastException;" => "Ljava/lang/RuntimeException;",
        "Ljava/lang/Exception;" => "Ljava/lang/Throwable;",
        "Ljava/lang/Error;" => "Ljava/lang/Throwable;",
        _ => return None,
    })
}

/// Alamat handler yang menangkap eksepsi pada `throw_pc`, bila ada.
fn find_catch_handler(
    dex: &DexFile,
    code: &CodeItem,
    throw_pc: u32,
    thrown_type: &str,
) -> Option<u32> {
    let item = code.tries.iter().find(|item| {
        let end = item
            .start_address
            .saturating_add(u32::from(item.instruction_count));
        throw_pc >= item.start_address && throw_pc < end
    })?;

    let handler = code
        .handlers
        .iter()
        .find(|handler| handler.offset == u32::from(item.handler_offset))?;

    for pair in &handler.pairs {
        let Some(catch_type) = dex.type_ids.get(pair.catch_type_idx as usize) else {
            continue;
        };
        if is_assignable_to(dex, thrown_type, catch_type) {
            return Some(pair.catch_address);
        }
    }

    handler.catch_all
}

/// Baca payload `fill-array-data` pada posisi `target` (dalam code unit).
///
/// Payload bukan instruksi biasa: isinya pseudo-opcode 0x0300, lebar elemen,
/// jumlah elemen, lalu data mentah little-endian.
fn read_array_payload(code: &CodeItem, target: usize) -> Result<Vec<Value>, DexError> {
    if unit_at(code, target)? != 0x0300 {
        return Err(DexError::InvalidArrayPayload { pc: target as u32 });
    }
    let width = usize::from(unit_at(code, target + 1)?);
    if !matches!(width, 1 | 2 | 4 | 8) {
        return Err(DexError::InvalidArrayPayload { pc: target as u32 });
    }
    let size =
        u32::from(unit_at(code, target + 2)?) | (u32::from(unit_at(code, target + 3)?) << 16);
    if size > MAX_ARRAY_LEN {
        return Err(DexError::LimitExceeded {
            table: "array-data",
            count: size,
            limit: MAX_ARRAY_LEN,
        });
    }

    let base = (target + 4) * 2;
    let mut values = Vec::with_capacity(size as usize);
    for index in 0..size as usize {
        let start = base + index * width;
        let mut bits = 0u64;
        for offset in 0..width {
            let position = start + offset;
            let unit = unit_at(code, position / 2)?;
            let byte = if position.is_multiple_of(2) {
                (unit & 0xFF) as u8
            } else {
                (unit >> 8) as u8
            };
            bits |= u64::from(byte) << (8 * offset);
        }
        values.push(match width {
            1 => Value::Int(i32::from(bits as u8 as i8)),
            2 => Value::Int(i32::from(bits as u16 as i16)),
            4 => Value::Int(bits as u32 as i32),
            _ => Value::Wide(bits),
        });
    }
    Ok(values)
}

fn read_reg(registers: &[Value], index: u32) -> Result<Value, DexError> {
    registers
        .get(index as usize)
        .copied()
        .ok_or(DexError::RegisterOutOfRange {
            register: index,
            registers: registers.len() as u16,
        })
}

/// Baca register sebagai referensi objek; nilai int dianggap referensi null.
fn read_ref(registers: &[Value], index: u32, pc: usize) -> Result<u32, DexError> {
    read_reg(registers, index)?
        .as_ref()
        .ok_or(DexError::NullReference { pc: pc as u32 })
}

/// Ubah hasil perbandingan menjadi -1/0/1 sesuai konvensi DEX.
fn cmp_ord(ordering: Option<std::cmp::Ordering>) -> i32 {
    match ordering {
        Some(std::cmp::Ordering::Less) => -1,
        Some(std::cmp::Ordering::Equal) => 0,
        Some(std::cmp::Ordering::Greater) => 1,
        None => 0,
    }
}

fn read_wide(registers: &[Value], index: u32) -> Result<u64, DexError> {
    read_reg(registers, index)?
        .as_wide()
        .ok_or(DexError::TypeMismatch {
            expected: "long/double",
        })
}

fn read_long(registers: &[Value], index: u32) -> Result<i64, DexError> {
    read_wide(registers, index).map(|bits| bits as i64)
}

fn read_float(registers: &[Value], index: u32) -> Result<f32, DexError> {
    read_int(registers, index).map(|bits| f32::from_bits(bits as u32))
}

fn read_double(registers: &[Value], index: u32) -> Result<f64, DexError> {
    read_wide(registers, index).map(f64::from_bits)
}

fn read_int(registers: &[Value], index: u32) -> Result<i32, DexError> {
    read_reg(registers, index)?
        .as_int()
        .ok_or(DexError::TypeMismatch { expected: "int" })
}

fn write_reg(registers: &mut [Value], index: u32, value: Value) -> Result<(), DexError> {
    let len = registers.len();
    let slot = registers
        .get_mut(index as usize)
        .ok_or(DexError::RegisterOutOfRange {
            register: index,
            registers: len as u16,
        })?;
    *slot = value;
    Ok(())
}

fn unit_at(code: &CodeItem, index: usize) -> Result<u16, DexError> {
    code.insns.get(index).copied().ok_or(DexError::Truncated)
}

fn branch_target(pc: usize, offset: i32) -> Result<usize, DexError> {
    // Offset cabang DEX relatif terhadap alamat instruksi itu sendiri, bukan
    // terhadap instruksi berikutnya seperti bytecode JVM.
    let target = pc as i64 + i64::from(offset);
    usize::try_from(target).map_err(|_| DexError::InvalidBranch {
        pc: pc as u32,
        offset,
    })
}

/// Nilai 32-bit bertanda dari dua code unit little-endian.
fn read_i32_units(code: &CodeItem, index: usize) -> Result<i32, DexError> {
    let low = u32::from(unit_at(code, index)?);
    let high = u32::from(unit_at(code, index + 1)?);
    Ok(((high << 16) | low) as i32)
}

/// Lengkapi galat batas langkah dengan method terdalam yang sedang berjalan.
///
/// Hanya frame terdalam yang mencatat lokasi: galat yang sudah berlabel
/// diteruskan apa adanya, sehingga jalur sukses tidak mengalokasikan apa pun.
fn annotate_limit(
    result: Result<FrameExit, DexError>,
    class: &str,
    method: &str,
) -> Result<FrameExit, DexError> {
    match result {
        Err(DexError::ExecutionLimit {
            steps,
            location: None,
        }) => Err(DexError::ExecutionLimit {
            steps,
            location: Some(format!("{class}->{method}")),
        }),
        other => other,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_frame(
    dex: &DexFile,
    heap: &mut Heap,
    statics: &mut HashMap<u32, Value>,
    classes: &mut HashMap<u32, Value>,
    state: &mut ClassState,
    native: Option<&dyn NativeResolver>,
    code: &CodeItem,
    args: &[Value],
    depth: usize,
    steps: &mut u64,
) -> Result<FrameExit, DexError> {
    if depth > MAX_CALL_DEPTH {
        return Err(DexError::CallDepthExceeded { depth });
    }

    let mut registers = vec![Value::Int(0); code.registers_size as usize];
    let start = usize::from(code.registers_size.saturating_sub(code.ins_size));
    for (offset, value) in args.iter().enumerate() {
        write_reg(&mut registers, (start + offset) as u32, *value)?;
    }

    let mut pc: usize = 0;
    let mut last_result: Option<Value> = None;
    let mut current_exception: Option<Value> = None;

    loop {
        if pc >= code.insns.len() {
            return Err(DexError::Truncated);
        }

        *steps += 1;
        if *steps > MAX_STEPS {
            return Err(DexError::ExecutionLimit {
                steps: *steps,
                location: None,
            });
        }

        // Eksepsi ditampung dulu agar pencarian handler terjadi di luar frame
        // besar milik step(), termasuk untuk eksepsi dari method lain.
        let mut thrown: Option<Value> = None;
        match step(
            dex,
            heap,
            statics,
            classes,
            state,
            native,
            code,
            &mut registers,
            pc,
            &mut last_result,
            &mut current_exception,
            depth,
            steps,
        )? {
            Step::Goto(next) => pc = next,
            Step::Return(value) => return Ok(FrameExit::Return(value)),
            Step::Throw(value) => thrown = Some(value),
            Step::Invoke { method_index, args } => {
                match call_method(
                    dex,
                    heap,
                    statics,
                    classes,
                    state,
                    native,
                    method_index,
                    &args,
                    depth,
                    steps,
                )? {
                    FrameExit::Return(value) => {
                        last_result = Some(value.unwrap_or(Value::Int(0)));
                        pc += 3;
                    }
                    // Eksepsi dari method lain memakai pc invoke ini sebagai
                    // titik lempar agar try/catch pemanggil ikut berlaku.
                    FrameExit::Throw(value) => thrown = Some(value),
                }
            }
        }

        if let Some(value) = thrown {
            let thrown_type = thrown_type_name(heap, value);
            match find_catch_handler(dex, code, pc as u32, &thrown_type) {
                Some(target) => {
                    current_exception = Some(value);
                    pc = target as usize;
                }
                None => return Ok(FrameExit::Throw(value)),
            }
        }
    }
}

/// Jalankan satu instruksi dan laporkan apa yang harus dilakukan frame.
///
/// Fungsi ini terpisah dari run_frame supaya semua cabang opcode tidak hidup
/// selama pemanggilan rekursif; hanya frame run_frame yang bertumpuk.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn step(
    dex: &DexFile,
    heap: &mut Heap,
    statics: &mut HashMap<u32, Value>,
    classes: &mut HashMap<u32, Value>,
    state: &mut ClassState,
    native: Option<&dyn NativeResolver>,
    code: &CodeItem,
    registers: &mut [Value],
    pc: usize,
    last_result: &mut Option<Value>,
    current_exception: &mut Option<Value>,
    depth: usize,
    steps: &mut u64,
) -> Result<Step, DexError> {
    let mut pc = pc;
    let unit = code.insns.get(pc).copied().ok_or(DexError::Truncated)?;
    let opcode = (unit & 0xFF) as u8;

    match opcode {
        0x00 => pc += 1,
        0x1c => {
            let reg = u32::from(unit >> 8);
            let type_index = u32::from(unit_at(code, pc + 1)?);
            let descriptor = dex
                .type_ids
                .get(type_index as usize)
                .ok_or(DexError::Truncated)?
                .clone();
            let value = match classes.get(&type_index) {
                Some(value) => *value,
                None => {
                    let value = heap.alloc(Object::Class { descriptor });
                    classes.insert(type_index, value);
                    value
                }
            };
            write_reg(registers, reg, value)?;
            pc += 2;
        }
        0x1d | 0x1e => {
            // monitor-enter/exit tidak berarti pada runtime satu thread.
            pc += 1;
        }
        0x1f => {
            let reg = u32::from(unit >> 8);
            let type_index = usize::from(unit_at(code, pc + 1)?);
            let target = dex
                .type_ids
                .get(type_index)
                .cloned()
                .ok_or(DexError::Truncated)?;
            let value = read_reg(registers, reg)?;
            if let Some(descriptor) = value_descriptor(heap, value)
                && !is_assignable_to(dex, &descriptor, &target)
            {
                let exception = heap.alloc(Object::Instance {
                    type_name: "Ljava/lang/ClassCastException;".to_owned(),
                    fields: HashMap::new(),
                });
                return Ok(Step::Throw(exception));
            }
            pc += 2;
        }
        0x20 => {
            let dst = u32::from((unit >> 8) & 0xF);
            let object_reg = u32::from((unit >> 12) & 0xF);
            let type_index = usize::from(unit_at(code, pc + 1)?);
            let target = dex
                .type_ids
                .get(type_index)
                .cloned()
                .ok_or(DexError::Truncated)?;
            let value = read_reg(registers, object_reg)?;
            let result = value_descriptor(heap, value)
                .is_some_and(|descriptor| is_assignable_to(dex, &descriptor, &target));
            write_reg(&mut *registers, dst, Value::Int(i32::from(result)))?;
            pc += 2;
        }
        0x01 | 0x04 | 0x07 => {
            let dst = u32::from((unit >> 8) & 0xF);
            let src = u32::from((unit >> 12) & 0xF);
            let value = read_reg(registers, src)?;
            write_reg(&mut *registers, dst, value)?;
            pc += 1;
        }
        0x02 | 0x05 | 0x08 => {
            let dst = u32::from(unit >> 8);
            let src = u32::from(unit_at(code, pc + 1)?);
            let value = read_reg(registers, src)?;
            write_reg(&mut *registers, dst, value)?;
            pc += 2;
        }
        0x03 | 0x06 | 0x09 => {
            let dst = u32::from(unit_at(code, pc + 1)?);
            let src = u32::from(unit_at(code, pc + 2)?);
            let value = read_reg(registers, src)?;
            write_reg(&mut *registers, dst, value)?;
            pc += 3;
        }
        0x0a..=0x0c => {
            let dst = u32::from(unit >> 8);
            let value = last_result.take().ok_or(DexError::InvalidInstruction {
                pc: pc as u32,
                opcode,
            })?;
            write_reg(&mut *registers, dst, value)?;
            pc += 1;
        }
        0x0d => {
            let dst = u32::from(unit >> 8);
            let value = current_exception
                .take()
                .ok_or(DexError::InvalidInstruction {
                    pc: pc as u32,
                    opcode,
                })?;
            write_reg(&mut *registers, dst, value)?;
            pc += 1;
        }
        0x0e => return Ok(Step::Return(None)),
        0x0f..=0x11 => {
            let reg = u32::from(unit >> 8);
            return Ok(Step::Return(Some(read_reg(registers, reg)?)));
        }
        0x12 => {
            let dst = u32::from((unit >> 8) & 0xF);
            let literal = ((unit >> 12) & 0xF) as u8 as i8;
            let literal = (literal << 4) >> 4;
            write_reg(&mut *registers, dst, Value::Int(i32::from(literal)))?;
            pc += 1;
        }
        0x13 => {
            let dst = u32::from(unit >> 8);
            let literal = unit_at(code, pc + 1)? as i16;
            write_reg(&mut *registers, dst, Value::Int(i32::from(literal)))?;
            pc += 2;
        }
        0x14 => {
            let dst = u32::from(unit >> 8);
            let low = u32::from(unit_at(code, pc + 1)?);
            let high = u32::from(unit_at(code, pc + 2)?);
            let literal = ((high << 16) | low) as i32;
            write_reg(&mut *registers, dst, Value::Int(literal))?;
            pc += 3;
        }
        0x15 => {
            let dst = u32::from(unit >> 8);
            let literal = i32::from(unit_at(code, pc + 1)? as i16) << 16;
            write_reg(&mut *registers, dst, Value::Int(literal))?;
            pc += 2;
        }
        0x16 => {
            let dst = u32::from(unit >> 8);
            let literal = i64::from(unit_at(code, pc + 1)? as i16);
            write_reg(&mut *registers, dst, Value::from_long(literal))?;
            pc += 2;
        }
        0x17 => {
            let dst = u32::from(unit >> 8);
            let low = u32::from(unit_at(code, pc + 1)?);
            let high = u32::from(unit_at(code, pc + 2)?);
            let literal = ((high << 16) | low) as i32;
            write_reg(&mut *registers, dst, Value::from_long(i64::from(literal)))?;
            pc += 3;
        }
        0x18 => {
            let dst = u32::from(unit >> 8);
            let mut bits = 0u64;
            for shift in 0..4 {
                bits |= u64::from(unit_at(code, pc + 1 + shift)?) << (16 * shift);
            }
            write_reg(&mut *registers, dst, Value::Wide(bits))?;
            pc += 5;
        }
        0x19 => {
            let dst = u32::from(unit >> 8);
            let literal = i64::from(unit_at(code, pc + 1)? as i16) << 48;
            write_reg(&mut *registers, dst, Value::from_long(literal))?;
            pc += 2;
        }
        0x1a | 0x1b => {
            // 0x1a memakai indeks 16-bit (21c); 0x1b memakai indeks 32-bit
            // (31c) sehingga butuh dua unit tambahan.
            let jumbo = unit & 0xFF == 0x1b;
            let dst = u32::from(unit >> 8);
            let string_index = if jumbo {
                usize::try_from(
                    u32::from(unit_at(code, pc + 1)?) | (u32::from(unit_at(code, pc + 2)?) << 16),
                )
                .map_err(|_| DexError::Truncated)?
            } else {
                usize::from(unit_at(code, pc + 1)?)
            };
            let text = dex
                .string_ids
                .get(string_index)
                .ok_or(DexError::Truncated)?
                .clone();
            let value = heap.alloc(Object::String(text));
            write_reg(&mut *registers, dst, value)?;
            pc += if jumbo { 3 } else { 2 };
        }
        0x21 => {
            let dst = u32::from((unit >> 8) & 0xF);
            let src = u32::from((unit >> 12) & 0xF);
            let reference = read_ref(registers, src, pc)?;
            let length = heap
                .array_len(reference)
                .ok_or(DexError::TypeMismatch { expected: "array" })?;
            write_reg(&mut *registers, dst, Value::Int(length as i32))?;
            pc += 1;
        }
        0x22 => {
            let dst = u32::from(unit >> 8);
            let type_index = usize::from(unit_at(code, pc + 1)?);
            let type_name = dex
                .type_ids
                .get(type_index)
                .ok_or(DexError::Truncated)?
                .clone();
            // JVM menginisialisasi class sebelum instance pertama dibuat.
            ensure_class_initialized(
                dex, heap, statics, classes, state, native, &type_name, depth, steps,
            )?;
            let value = heap.alloc(Object::Instance {
                type_name,
                fields: HashMap::new(),
            });
            write_reg(&mut *registers, dst, value)?;
            pc += 2;
        }
        0x23 => {
            let dst = u32::from((unit >> 8) & 0xF);
            let size_reg = u32::from((unit >> 12) & 0xF);
            let type_index = usize::from(unit_at(code, pc + 1)?);
            let size = read_int(registers, size_reg)?;
            if size < 0 {
                return Err(DexError::NegativeArraySize { size });
            }
            let size = size as u32;
            if size > MAX_ARRAY_LEN {
                return Err(DexError::LimitExceeded {
                    table: "array",
                    count: size,
                    limit: MAX_ARRAY_LEN,
                });
            }
            let element_type = dex
                .type_ids
                .get(type_index)
                .ok_or(DexError::Truncated)?
                .clone();
            let value = heap.alloc(Object::Array {
                element_type,
                elements: vec![Value::Int(0); size as usize],
            });
            write_reg(&mut *registers, dst, value)?;
            pc += 2;
        }
        0x24 => {
            let count = usize::from((unit >> 12) & 0xF);
            let fifth = u32::from((unit >> 8) & 0xF);
            let type_index = usize::from(unit_at(code, pc + 1)?);
            let packed = unit_at(code, pc + 2)?;
            let arg_registers = [
                u32::from(packed & 0xF),
                u32::from((packed >> 4) & 0xF),
                u32::from((packed >> 8) & 0xF),
                u32::from((packed >> 12) & 0xF),
                fifth,
            ];
            let element_type = dex
                .type_ids
                .get(type_index)
                .cloned()
                .ok_or(DexError::Truncated)?;
            let mut elements = Vec::with_capacity(count);
            for register in arg_registers.iter().take(count) {
                elements.push(read_reg(registers, *register)?);
            }
            let value = heap.alloc(Object::Array {
                element_type,
                elements,
            });
            *last_result = Some(value);
            pc += 3;
        }
        0x25 => {
            let count = usize::from(unit >> 8);
            let type_index = usize::from(unit_at(code, pc + 1)?);
            let first = u32::from(unit_at(code, pc + 2)?);
            let element_type = dex
                .type_ids
                .get(type_index)
                .cloned()
                .ok_or(DexError::Truncated)?;
            let mut elements = Vec::with_capacity(count);
            for offset in 0..count {
                elements.push(read_reg(registers, first + offset as u32)?);
            }
            let value = heap.alloc(Object::Array {
                element_type,
                elements,
            });
            *last_result = Some(value);
            pc += 3;
        }
        0x26 => {
            let reg = u32::from(unit >> 8);
            let low = u32::from(unit_at(code, pc + 1)?);
            let high = u32::from(unit_at(code, pc + 2)?);
            let offset = ((high << 16) | low) as i32;
            let reference = read_ref(registers, reg, pc)?;
            let payload_pc = branch_target(pc, offset)?;
            let values = read_array_payload(code, payload_pc)?;
            match heap.get_mut(reference) {
                Some(Object::Array { elements, .. }) => {
                    if elements.len() != values.len() {
                        return Err(DexError::ArrayDataMismatch {
                            payload: values.len(),
                            array: elements.len(),
                        });
                    }
                    for (slot, value) in elements.iter_mut().zip(values) {
                        *slot = value;
                    }
                }
                _ => return Err(DexError::TypeMismatch { expected: "array" }),
            }
            pc += 3;
        }
        0x27 => {
            let reg = u32::from(unit >> 8);
            let value = read_reg(registers, reg)?;
            if value.as_ref().is_none() {
                return Err(DexError::ThrowNonReference { pc: pc as u32 });
            }
            return Ok(Step::Throw(value));
        }
        0x28 => {
            let offset = (unit >> 8) as u8 as i8;
            pc = branch_target(pc, i32::from(offset))?;
        }
        0x29 => {
            let offset = unit_at(code, pc + 1)? as i16;
            pc = branch_target(pc, i32::from(offset))?;
        }
        0x2a => {
            let low = u32::from(unit_at(code, pc + 1)?);
            let high = u32::from(unit_at(code, pc + 2)?);
            let offset = ((high << 16) | low) as i32;
            pc = branch_target(pc, offset)?;
        }
        0x2d..=0x31 => {
            let dst = u32::from(unit >> 8);
            let packed = unit_at(code, pc + 1)?;
            let left = u32::from(packed & 0xFF);
            let right = u32::from(packed >> 8);
            // cmpl mengembalikan -1 dan cmpg mengembalikan 1 untuk NaN.
            let result = match opcode {
                0x2d => {
                    let a = read_float(registers, left)?;
                    let b = read_float(registers, right)?;
                    if a.is_nan() || b.is_nan() {
                        -1
                    } else {
                        cmp_ord(a.partial_cmp(&b))
                    }
                }
                0x2e => {
                    let a = read_float(registers, left)?;
                    let b = read_float(registers, right)?;
                    if a.is_nan() || b.is_nan() {
                        1
                    } else {
                        cmp_ord(a.partial_cmp(&b))
                    }
                }
                0x2f => {
                    let a = read_double(registers, left)?;
                    let b = read_double(registers, right)?;
                    if a.is_nan() || b.is_nan() {
                        -1
                    } else {
                        cmp_ord(a.partial_cmp(&b))
                    }
                }
                0x30 => {
                    let a = read_double(registers, left)?;
                    let b = read_double(registers, right)?;
                    if a.is_nan() || b.is_nan() {
                        1
                    } else {
                        cmp_ord(a.partial_cmp(&b))
                    }
                }
                _ => cmp_ord(Some(
                    read_long(registers, left)?.cmp(&read_long(registers, right)?),
                )),
            };
            write_reg(&mut *registers, dst, Value::Int(result))?;
            pc += 2;
        }
        0x32..=0x37 => {
            let a = u32::from((unit >> 8) & 0xF);
            let b = u32::from((unit >> 12) & 0xF);
            let offset = unit_at(code, pc + 1)? as i16;
            // if-eq/if-ne juga dipakai untuk membandingkan referensi objek,
            // jadi kedua opcode itu membandingkan nilai apa adanya.
            let taken = match opcode {
                0x32 => read_reg(registers, a)? == read_reg(registers, b)?,
                0x33 => read_reg(registers, a)? != read_reg(registers, b)?,
                _ => {
                    let left = read_int(registers, a)?;
                    let right = read_int(registers, b)?;
                    match opcode {
                        0x34 => left < right,
                        0x35 => left >= right,
                        0x36 => left > right,
                        _ => left <= right,
                    }
                }
            };
            pc = if taken {
                branch_target(pc, i32::from(offset))?
            } else {
                pc + 2
            };
        }
        0x38..=0x3d => {
            let a = u32::from(unit >> 8);
            let offset = unit_at(code, pc + 1)? as i16;
            // `null` diwakili `Int(0)`; referensi objek selalu bukan null.
            let value = read_reg(registers, a)?;
            let taken = match opcode {
                0x38 => value == Value::Int(0),
                0x39 => value != Value::Int(0),
                _ => {
                    let value = read_int(registers, a)?;
                    match opcode {
                        0x3a => value < 0,
                        0x3b => value >= 0,
                        0x3c => value > 0,
                        _ => value <= 0,
                    }
                }
            };
            pc = if taken {
                branch_target(pc, i32::from(offset))?
            } else {
                pc + 2
            };
        }
        0x44..=0x4a => {
            let dst = u32::from(unit >> 8);
            let packed = unit_at(code, pc + 1)?;
            let array_reg = u32::from(packed & 0xFF);
            let index_reg = u32::from(packed >> 8);
            let reference = read_ref(registers, array_reg, pc)?;
            let index = read_int(registers, index_reg)?;
            let value = match heap.get(reference) {
                Some(Object::Array { elements, .. }) => {
                    let length = elements.len();
                    usize::try_from(index)
                        .ok()
                        .and_then(|slot| elements.get(slot).copied())
                        .ok_or(DexError::ArrayIndexOutOfBounds { index, length })?
                }
                _ => return Err(DexError::TypeMismatch { expected: "array" }),
            };
            write_reg(&mut *registers, dst, value)?;
            pc += 2;
        }
        0x4b..=0x51 => {
            let src = u32::from(unit >> 8);
            let packed = unit_at(code, pc + 1)?;
            let array_reg = u32::from(packed & 0xFF);
            let index_reg = u32::from(packed >> 8);
            let value = read_reg(registers, src)?;
            let reference = read_ref(registers, array_reg, pc)?;
            let index = read_int(registers, index_reg)?;
            match heap.get_mut(reference) {
                Some(Object::Array { elements, .. }) => {
                    let length = elements.len();
                    match usize::try_from(index)
                        .ok()
                        .and_then(|slot| elements.get_mut(slot))
                    {
                        Some(slot) => *slot = value,
                        None => {
                            return Err(DexError::ArrayIndexOutOfBounds { index, length });
                        }
                    }
                }
                _ => return Err(DexError::TypeMismatch { expected: "array" }),
            }
            pc += 2;
        }
        0x52..=0x58 => {
            let dst = u32::from((unit >> 8) & 0xF);
            let object_reg = u32::from((unit >> 12) & 0xF);
            let field_index = u32::from(unit_at(code, pc + 1)?);
            let reference = read_ref(registers, object_reg, pc)?;
            let stored = match heap.get(reference) {
                Some(Object::Instance { fields, .. }) => Some(fields.get(&field_index).copied()),
                _ => None,
            };
            let field = dex
                .field_ids
                .get(field_index as usize)
                .ok_or(DexError::FieldNotFound { field_index })?;
            let value = match stored {
                Some(Some(value)) => value,
                // Field framework diisi runtime saat belum pernah ditulis.
                Some(None) => native
                    .and_then(|resolver| resolver.field_default(heap, &field.class, &field.name))
                    .unwrap_or_else(|| zero_for_field_type(&field.field_type)),
                None => {
                    return Err(DexError::TypeMismatch {
                        expected: "instance",
                    });
                }
            };
            write_reg(&mut *registers, dst, value)?;
            pc += 2;
        }
        0x59..=0x5f => {
            let src = u32::from((unit >> 8) & 0xF);
            let object_reg = u32::from((unit >> 12) & 0xF);
            let field_index = u32::from(unit_at(code, pc + 1)?);
            let value = read_reg(registers, src)?;
            let reference = read_ref(registers, object_reg, pc)?;
            match heap.get_mut(reference) {
                Some(Object::Instance { fields, .. }) => {
                    fields.insert(field_index, value);
                }
                _ => {
                    return Err(DexError::TypeMismatch {
                        expected: "instance",
                    });
                }
            }
            pc += 2;
        }
        0x60..=0x66 => {
            let dst = u32::from(unit >> 8);
            let field_index = u32::from(unit_at(code, pc + 1)?);
            if field_index as usize >= dex.field_ids.len() {
                return Err(DexError::FieldNotFound { field_index });
            }
            let field_type = &dex.field_ids[field_index as usize].field_type;
            let class = dex.field_ids[field_index as usize].class.as_str();
            ensure_class_initialized(
                dex, heap, statics, classes, state, native, class, depth, steps,
            )?;
            let value = statics
                .get(&field_index)
                .copied()
                .unwrap_or_else(|| zero_for_field_type(field_type));
            if trace_statics() {
                let field = &dex.field_ids[field_index as usize];
                eprintln!("[static] get {};->{} = {value:?}", field.class, field.name);
            }
            write_reg(&mut *registers, dst, value)?;
            pc += 2;
        }
        0x67..=0x6d => {
            let src = u32::from(unit >> 8);
            let field_index = u32::from(unit_at(code, pc + 1)?);
            if field_index as usize >= dex.field_ids.len() {
                return Err(DexError::FieldNotFound { field_index });
            }
            let value = read_reg(registers, src)?;
            let class = dex.field_ids[field_index as usize].class.as_str();
            ensure_class_initialized(
                dex, heap, statics, classes, state, native, class, depth, steps,
            )?;
            if trace_statics() {
                let field = &dex.field_ids[field_index as usize];
                eprintln!("[static] put {};->{} = {value:?}", field.class, field.name);
            }
            statics.insert(field_index, value);
            pc += 2;
        }
        0x7b..=0x8f => {
            let dst = u32::from((unit >> 8) & 0xF);
            let src = u32::from((unit >> 12) & 0xF);
            let value = match opcode {
                0x7b => Value::Int(read_int(registers, src)?.wrapping_neg()),
                0x7c => Value::Int(!read_int(registers, src)?),
                0x7d => Value::from_long(read_long(registers, src)?.wrapping_neg()),
                0x7e => Value::Wide(!read_wide(registers, src)?),
                0x7f => Value::from_float(-read_float(registers, src)?),
                0x80 => Value::from_double(-read_double(registers, src)?),
                0x81 => Value::from_long(i64::from(read_int(registers, src)?)),
                0x82 => Value::from_float(read_int(registers, src)? as f32),
                0x83 => Value::from_double(f64::from(read_int(registers, src)?)),
                0x84 => Value::Int(read_long(registers, src)? as i32),
                0x85 => Value::from_float(read_long(registers, src)? as f32),
                0x86 => Value::from_double(read_long(registers, src)? as f64),
                0x87 => Value::Int(read_float(registers, src)? as i32),
                0x88 => Value::from_long(read_float(registers, src)? as i64),
                0x89 => Value::from_double(f64::from(read_float(registers, src)?)),
                0x8a => Value::Int(read_double(registers, src)? as i32),
                0x8b => Value::from_long(read_double(registers, src)? as i64),
                0x8c => Value::from_float(read_double(registers, src)? as f32),
                0x8d => Value::Int(i32::from(read_int(registers, src)? as i8)),
                0x8e => Value::Int(i32::from(read_int(registers, src)? as u16)),
                _ => Value::Int(i32::from(read_int(registers, src)? as i16)),
            };
            write_reg(&mut *registers, dst, value)?;
            pc += 1;
        }
        0x90..=0x9a => {
            let dst = u32::from(unit >> 8);
            let next = unit_at(code, pc + 1)?;
            let left = u32::from(next & 0xFF);
            let right = u32::from(next >> 8);
            let a = read_int(registers, left)?;
            let b = read_int(registers, right)?;
            let result = match opcode {
                0x90 => a.wrapping_add(b),
                0x91 => a.wrapping_sub(b),
                0x92 => a.wrapping_mul(b),
                0x93 => {
                    if b == 0 {
                        return Err(DexError::DivisionByZero);
                    }
                    a.wrapping_div(b)
                }
                0x94 => {
                    if b == 0 {
                        return Err(DexError::DivisionByZero);
                    }
                    a.wrapping_rem(b)
                }
                0x95 => a & b,
                0x96 => a | b,
                0x97 => a ^ b,
                0x98 => a.wrapping_shl((b & 0x1F) as u32),
                0x99 => a.wrapping_shr((b & 0x1F) as u32),
                _ => ((a as u32) >> (b & 0x1F)) as i32,
            };
            write_reg(&mut *registers, dst, Value::Int(result))?;
            pc += 2;
        }
        0x9b..=0xa5 => {
            let dst = u32::from(unit >> 8);
            let packed = unit_at(code, pc + 1)?;
            let left = u32::from(packed & 0xFF);
            let right = u32::from(packed >> 8);
            let a = read_long(registers, left)?;
            let b = read_long(registers, right)?;
            let result = match opcode {
                0x9b => a.wrapping_add(b),
                0x9c => a.wrapping_sub(b),
                0x9d => a.wrapping_mul(b),
                0x9e => {
                    if b == 0 {
                        return Err(DexError::DivisionByZero);
                    }
                    a.wrapping_div(b)
                }
                0x9f => {
                    if b == 0 {
                        return Err(DexError::DivisionByZero);
                    }
                    a.wrapping_rem(b)
                }
                0xa0 => a & b,
                0xa1 => a | b,
                0xa2 => a ^ b,
                0xa3 => a.wrapping_shl((b & 0x3F) as u32),
                0xa4 => a.wrapping_shr((b & 0x3F) as u32),
                _ => ((a as u64) >> (b & 0x3F)) as i64,
            };
            write_reg(&mut *registers, dst, Value::from_long(result))?;
            pc += 2;
        }
        0xa6..=0xaa => {
            let dst = u32::from(unit >> 8);
            let packed = unit_at(code, pc + 1)?;
            let left = u32::from(packed & 0xFF);
            let right = u32::from(packed >> 8);
            let a = read_float(registers, left)?;
            let b = read_float(registers, right)?;
            let result = match opcode {
                0xa6 => a + b,
                0xa7 => a - b,
                0xa8 => a * b,
                0xa9 => a / b,
                _ => a % b,
            };
            write_reg(&mut *registers, dst, Value::from_float(result))?;
            pc += 2;
        }
        0xab..=0xaf => {
            let dst = u32::from(unit >> 8);
            let packed = unit_at(code, pc + 1)?;
            let left = u32::from(packed & 0xFF);
            let right = u32::from(packed >> 8);
            let a = read_double(registers, left)?;
            let b = read_double(registers, right)?;
            let result = match opcode {
                0xab => a + b,
                0xac => a - b,
                0xad => a * b,
                0xae => a / b,
                _ => a % b,
            };
            write_reg(&mut *registers, dst, Value::from_double(result))?;
            pc += 2;
        }
        // Varian 12x: vA menjadi tujuan sekaligus operand kiri.
        0xb0..=0xcf => {
            let dst = u32::from((unit >> 8) & 0xF);
            let src = u32::from((unit >> 12) & 0xF);
            let value = match opcode {
                0xb0..=0xba => {
                    let a = read_int(registers, dst)?;
                    let b = read_int(registers, src)?;
                    let result = match opcode {
                        0xb0 => a.wrapping_add(b),
                        0xb1 => a.wrapping_sub(b),
                        0xb2 => a.wrapping_mul(b),
                        0xb3 => {
                            if b == 0 {
                                return Err(DexError::DivisionByZero);
                            }
                            a.wrapping_div(b)
                        }
                        0xb4 => {
                            if b == 0 {
                                return Err(DexError::DivisionByZero);
                            }
                            a.wrapping_rem(b)
                        }
                        0xb5 => a & b,
                        0xb6 => a | b,
                        0xb7 => a ^ b,
                        0xb8 => a.wrapping_shl((b & 0x1F) as u32),
                        0xb9 => a.wrapping_shr((b & 0x1F) as u32),
                        _ => ((a as u32) >> (b & 0x1F)) as i32,
                    };
                    Value::Int(result)
                }
                0xbb..=0xc5 => {
                    let a = read_long(registers, dst)?;
                    let b = read_long(registers, src)?;
                    let result = match opcode {
                        0xbb => a.wrapping_add(b),
                        0xbc => a.wrapping_sub(b),
                        0xbd => a.wrapping_mul(b),
                        0xbe => {
                            if b == 0 {
                                return Err(DexError::DivisionByZero);
                            }
                            a.wrapping_div(b)
                        }
                        0xbf => {
                            if b == 0 {
                                return Err(DexError::DivisionByZero);
                            }
                            a.wrapping_rem(b)
                        }
                        0xc0 => a & b,
                        0xc1 => a | b,
                        0xc2 => a ^ b,
                        0xc3 => a.wrapping_shl((b & 0x3F) as u32),
                        0xc4 => a.wrapping_shr((b & 0x3F) as u32),
                        _ => ((a as u64) >> (b & 0x3F)) as i64,
                    };
                    Value::from_long(result)
                }
                0xc6..=0xca => {
                    let a = read_float(registers, dst)?;
                    let b = read_float(registers, src)?;
                    let result = match opcode {
                        0xc6 => a + b,
                        0xc7 => a - b,
                        0xc8 => a * b,
                        0xc9 => a / b,
                        _ => a % b,
                    };
                    Value::from_float(result)
                }
                _ => {
                    let a = read_double(registers, dst)?;
                    let b = read_double(registers, src)?;
                    let result = match opcode {
                        0xcb => a + b,
                        0xcc => a - b,
                        0xcd => a * b,
                        0xce => a / b,
                        _ => a % b,
                    };
                    Value::from_double(result)
                }
            };
            write_reg(&mut *registers, dst, value)?;
            pc += 1;
        }
        // 0x6f invoke-super memakai method ref yang sudah menunjuk superclass.
        0x6e..=0x72 => {
            let method_index = u32::from(unit_at(code, pc + 1)?);
            let arg_count = usize::from((unit >> 12) & 0xF);
            let fifth = u32::from((unit >> 8) & 0xF);
            let packed = unit_at(code, pc + 2)?;

            let arg_registers = [
                u32::from(packed & 0xF),
                u32::from((packed >> 4) & 0xF),
                u32::from((packed >> 8) & 0xF),
                u32::from((packed >> 12) & 0xF),
                fifth,
            ];

            if opcode == 0x71 {
                let class = dex
                    .method_ids
                    .get(method_index as usize)
                    .ok_or(DexError::Truncated)?
                    .class
                    .as_str();
                ensure_class_initialized(
                    dex, heap, statics, classes, state, native, class, depth, steps,
                )?;
            }
            let values = collect_invoke_args(
                dex,
                registers,
                method_index,
                arg_count,
                arg_registers.into_iter().take(arg_count),
            )?;
            return Ok(Step::Invoke {
                method_index,
                args: values,
            });
        }
        // Varian /range memakai register berurutan mulai dari CCCC.
        // 0x73 invoke-virtual/range, 0x74 invoke-super/range, 0x75
        // invoke-direct/range, 0x76 invoke-interface/range, 0x77
        // invoke-static/range.
        0x73..=0x78 => {
            let method_index = u32::from(unit_at(code, pc + 1)?);
            let arg_count = usize::from(unit >> 8);
            let first = u32::from(unit_at(code, pc + 2)?);
            if opcode == 0x77 {
                let class = dex
                    .method_ids
                    .get(method_index as usize)
                    .ok_or(DexError::Truncated)?
                    .class
                    .as_str();
                ensure_class_initialized(
                    dex, heap, statics, classes, state, native, class, depth, steps,
                )?;
            }
            let values = collect_invoke_args(
                dex,
                registers,
                method_index,
                arg_count,
                (0..arg_count as u32).map(|offset| first + offset),
            )?;
            return Ok(Step::Invoke {
                method_index,
                args: values,
            });
        }
        // add-int/lit16 .. xor-int/lit16 (format 22s): vAA, vB, literal#CCCC.
        0xd0..=0xd7 => {
            let dst = u32::from(unit >> 8);
            let next = unit_at(code, pc + 1)?;
            let src = u32::from(next >> 12);
            let literal = next as i16;
            let value = read_int(registers, src)?;
            let literal = i32::from(literal);
            let result = match opcode {
                0xd0 => value.wrapping_add(literal),
                0xd1 => literal.wrapping_sub(value),
                0xd2 => value.wrapping_mul(literal),
                0xd3 => {
                    if literal == 0 {
                        return Err(DexError::DivisionByZero);
                    }
                    value.wrapping_div(literal)
                }
                0xd4 => {
                    if literal == 0 {
                        return Err(DexError::DivisionByZero);
                    }
                    value.wrapping_rem(literal)
                }
                0xd5 => value & literal,
                0xd6 => value | literal,
                _ => value ^ literal,
            };
            write_reg(registers, dst, Value::Int(result))?;
            pc += 2;
        }
        // add-int/lit8 .. ushr-int/lit8 (format 22b): vAA, vBB, literal#CC.
        0xd8..=0xe2 => {
            let dst = u32::from(unit >> 8);
            let next = unit_at(code, pc + 1)?;
            let src = u32::from(next & 0xFF);
            let literal = i32::from((next >> 8) as u8 as i8);
            let value = read_int(registers, src)?;
            let result = match opcode {
                0xd8 => value.wrapping_add(literal),
                0xd9 => literal.wrapping_sub(value),
                0xda => value.wrapping_mul(literal),
                0xdb => {
                    if literal == 0 {
                        return Err(DexError::DivisionByZero);
                    }
                    value.wrapping_div(literal)
                }
                0xdc => {
                    if literal == 0 {
                        return Err(DexError::DivisionByZero);
                    }
                    value.wrapping_rem(literal)
                }
                0xdd => value & literal,
                0xde => value | literal,
                0xdf => value ^ literal,
                0xe0 => value.wrapping_shl((literal & 0x1F) as u32),
                0xe1 => value.wrapping_shr((literal & 0x1F) as u32),
                _ => ((value as u32) >> (literal & 0x1F)) as i32,
            };
            write_reg(registers, dst, Value::Int(result))?;
            pc += 2;
        }
        // packed-switch dan sparse-switch (format 31t).
        //
        // Payload berada di belakang kode, dan target di dalamnya relatif
        // terhadap instruksi switch (bukan terhadap payload). Kunci yang tidak
        // cocok melanjutkan eksekusi ke instruksi setelah switch.
        0x2b | 0x2c => {
            let key = read_int(registers, u32::from(unit >> 8))?;
            let offset = read_i32_units(code, pc + 1)?;
            let payload_pc = branch_target(pc, offset)?;
            let ident = unit_at(code, payload_pc)?;
            let expected = if opcode == 0x2b { 0x0100 } else { 0x0200 };
            if ident != expected {
                return Err(DexError::InvalidInstruction {
                    pc: payload_pc as u32,
                    opcode: (ident & 0xFF) as u8,
                });
            }
            let size = usize::from(unit_at(code, payload_pc + 1)?);
            let target = if opcode == 0x2b {
                let first_key = read_i32_units(code, payload_pc + 2)?;
                let last_key = first_key.saturating_add(size as i32);
                if key < first_key || key >= last_key {
                    None
                } else {
                    let index = (key - first_key) as usize;
                    Some(read_i32_units(code, payload_pc + 4 + index * 2)?)
                }
            } else {
                let mut matched = None;
                for index in 0..size {
                    let candidate = read_i32_units(code, payload_pc + 2 + index * 2)?;
                    if candidate == key {
                        matched = Some(read_i32_units(code, payload_pc + 2 + (size + index) * 2)?);
                        break;
                    }
                }
                matched
            };
            match target {
                Some(relative) => pc = branch_target(pc, relative)?,
                None => pc += 3,
            }
        }
        _ => {
            return Err(DexError::InvalidInstruction {
                pc: pc as u32,
                opcode,
            });
        }
    }

    Ok(Step::Goto(pc))
}

/// Diagnostik akses field statis, aktif bila ARUNIKA_TRACE_STATICS diset.
fn trace_statics() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("ARUNIKA_TRACE_STATICS").is_some())
}

/// Jalankan `<clinit>` sebuah class sekali sebelum penggunaan aktif pertama.
///
/// JVM menginisialisasi class saat `new-instance`, akses field statis, atau
/// pemanggilan method statis pertama. `state` mencegah pengulangan dan
/// memutus rekursi ketika initializer memakai class-nya sendiri.
#[allow(clippy::too_many_arguments)]
fn ensure_class_initialized(
    dex: &DexFile,
    heap: &mut Heap,
    statics: &mut HashMap<u32, Value>,
    classes: &mut HashMap<u32, Value>,
    state: &mut ClassState,
    native: Option<&dyn NativeResolver>,
    class: &str,
    depth: usize,
    steps: &mut u64,
) -> Result<(), DexError> {
    if state.initialized.contains(class) || state.initializing.contains(class) {
        return Ok(());
    }
    let Some(index) = dex.method_index(class, "<clinit>") else {
        state.initialized.insert(class.to_string());
        return Ok(());
    };
    if dex.code_item_for_method(index as u32).is_none() {
        state.initialized.insert(class.to_string());
        return Ok(());
    }
    state.initializing.insert(class.to_string());
    let exit = call_method(
        dex,
        heap,
        statics,
        classes,
        state,
        native,
        index as u32,
        &[],
        depth + 1,
        steps,
    );
    state.initializing.remove(class);
    state.initialized.insert(class.to_string());
    match exit? {
        FrameExit::Return(_) => Ok(()),
        FrameExit::Throw(value) => Err(DexError::UncaughtException {
            type_name: thrown_type_name(heap, value),
        }),
    }
}

/// Susun argumen invoke dari register sesuai proto method.
///
/// `this` implisit diambil lebih dulu bila jumlah register melebihi jumlah
/// word parameter, dan word kedua long/double dilewati karena nilainya
/// disimpan utuh pada register pertama.
fn collect_invoke_args(
    dex: &DexFile,
    registers: &[Value],
    method_index: u32,
    arg_count: usize,
    mut used: impl Iterator<Item = u32>,
) -> Result<Vec<Value>, DexError> {
    let mut values = Vec::with_capacity(arg_count);
    match dex.method_parameter_types(method_index) {
        Some(parameters) => {
            let parameter_words: usize = parameters
                .iter()
                .map(|parameter| {
                    if matches!(parameter.as_str(), "J" | "D") {
                        2
                    } else {
                        1
                    }
                })
                .sum();
            if arg_count > parameter_words
                && let Some(register) = used.next()
            {
                values.push(read_reg(registers, register)?);
            }
            for parameter in parameters {
                let Some(register) = used.next() else {
                    break;
                };
                values.push(read_reg(registers, register)?);
                if matches!(parameter.as_str(), "J" | "D") {
                    used.next();
                }
            }
        }
        None => {
            for register in used {
                values.push(read_reg(registers, register)?);
            }
        }
    }
    Ok(values)
}

/// Nilai balik `Class.getMethod` dan `Method.invoke`.
///
/// Reflection butuh akses ke tabel method DEX, jadi ditangani interpreter
/// alih-alih resolver native: `invoke` harus masuk kembali ke `call_method`
/// dengan frame yang sama seperti pemanggilan biasa.
#[allow(clippy::too_many_arguments)]
fn reflect_call(
    dex: &DexFile,
    heap: &mut Heap,
    statics: &mut HashMap<u32, Value>,
    classes: &mut HashMap<u32, Value>,
    state: &mut ClassState,
    native: Option<&dyn NativeResolver>,
    class: &str,
    name: &str,
    args: &[Value],
    depth: usize,
    steps: &mut u64,
) -> Option<Result<FrameExit, DexError>> {
    match (class, name) {
        ("Ljava/lang/Class;", "getMethod") | ("Ljava/lang/Class;", "getDeclaredMethod") => {
            Some((|| {
                let receiver = args
                    .first()
                    .and_then(|value| value.as_ref())
                    .ok_or(DexError::Truncated)?;
                let target = match heap.get(receiver) {
                    Some(Object::Class { descriptor }) => descriptor.clone(),
                    _ => {
                        return Err(DexError::TypeMismatch { expected: "class" });
                    }
                };
                let method_name = arg_text(heap, args.get(1)).unwrap_or_default();
                let parameters = arg_class_array(heap, args.get(2));
                if dex
                    .find_method_by_parameters(&target, &method_name, &parameters)
                    .is_some()
                {
                    Ok(FrameExit::Return(Some(heap.alloc(Object::Method {
                        class: target,
                        name: method_name,
                        parameters,
                    }))))
                } else {
                    let message = format!("{target}#{method_name}");
                    Ok(FrameExit::Throw(alloc_throwable(
                        heap,
                        "Ljava/lang/NoSuchMethodException;",
                        &message,
                    )))
                }
            })())
        }
        ("Ljava/lang/reflect/Method;", "invoke") => Some((|| {
            let receiver = args
                .first()
                .and_then(|value| value.as_ref())
                .ok_or(DexError::Truncated)?;
            let (target_class, target_name, parameters) = match heap.get(receiver) {
                Some(Object::Method {
                    class,
                    name,
                    parameters,
                }) => (class.clone(), name.clone(), parameters.clone()),
                _ => {
                    return Err(DexError::TypeMismatch { expected: "method" });
                }
            };
            let Some(index) =
                dex.find_method_by_parameters(&target_class, &target_name, &parameters)
            else {
                let message = format!("{target_class}#{target_name}");
                return Ok(FrameExit::Throw(alloc_throwable(
                    heap,
                    "Ljava/lang/NoSuchMethodException;",
                    &message,
                )));
            };

            let mut values = Vec::new();
            if let Some(target) = args.get(1).copied()
                && target.as_ref().is_some()
            {
                values.push(target);
            }
            if let Some(array) = args.get(2).and_then(|value| value.as_ref())
                && let Some(Object::Array { elements, .. }) = heap.get(array)
            {
                let elements = elements.clone();
                values.extend(elements.into_iter().map(|value| unbox(heap, value)));
            }

            call_method(
                dex, heap, statics, classes, state, native, index, &values, depth, steps,
            )
        })()),
        _ => None,
    }
}

fn arg_text(heap: &Heap, value: Option<&Value>) -> Option<String> {
    value
        .and_then(|value| value.as_ref())
        .and_then(|reference| heap.string(reference))
        .map(|text| text.to_owned())
}

fn arg_class_array(heap: &Heap, value: Option<&Value>) -> Vec<String> {
    let Some(reference) = value.and_then(|value| value.as_ref()) else {
        return Vec::new();
    };
    let Some(Object::Array { elements, .. }) = heap.get(reference) else {
        return Vec::new();
    };
    elements
        .iter()
        .filter_map(|element| {
            let reference = element.as_ref()?;
            match heap.get(reference) {
                Some(Object::Class { descriptor }) => Some(descriptor.clone()),
                _ => None,
            }
        })
        .collect()
}

/// Nilai primitif yang di-box dilepas saat masuk ke parameter method.
fn unbox(heap: &Heap, value: Value) -> Value {
    match value.as_ref().and_then(|reference| heap.get(reference)) {
        Some(Object::Boxed { value, .. }) => *value,
        _ => value,
    }
}

fn alloc_throwable(heap: &mut Heap, class: &str, message: &str) -> Value {
    let text = heap.alloc(Object::String(message.to_string()));
    let mut fields = HashMap::new();
    fields.insert(0, text);
    heap.alloc(Object::Instance {
        type_name: class.to_string(),
        fields,
    })
}

#[allow(clippy::too_many_arguments)]
fn call_method(
    dex: &DexFile,
    heap: &mut Heap,
    statics: &mut HashMap<u32, Value>,
    classes: &mut HashMap<u32, Value>,
    state: &mut ClassState,
    native: Option<&dyn NativeResolver>,
    method_index: u32,
    args: &[Value],
    depth: usize,
    steps: &mut u64,
) -> Result<FrameExit, DexError> {
    if let Some(callee) = dex.code_item_for_method(method_index) {
        let callee = callee.clone();
        let result = run_frame(
            dex,
            heap,
            statics,
            classes,
            state,
            native,
            &callee,
            args,
            depth + 1,
            steps,
        );
        return match dex.method_ids.get(method_index as usize) {
            Some(method) => annotate_limit(result, &method.class, &method.name),
            None => result,
        };
    }

    let method = dex
        .method_ids
        .get(method_index as usize)
        .ok_or(DexError::Truncated)?;

    let signature = dex.method_signature(method_index);

    // Reflection ditangani interpreter karena butuh tabel method DEX dan
    // kemampuan memanggil balik ke frame baru.
    if let Some(outcome) = reflect_call(
        dex,
        heap,
        statics,
        classes,
        state,
        native,
        &method.class,
        &method.name,
        args,
        depth,
        steps,
    ) {
        return outcome;
    }

    // Diagnostik opt-in: jejak pemanggilan API native yang tidak punya kode.
    if std::env::var_os("ARUNIKA_TRACE").is_some() {
        eprintln!(
            "[trace] {}{}{} args={args:?}",
            method.class, method.name, signature
        );
    }
    if let Some(resolver) = native
        && let Some(result) =
            resolver.invoke_described(heap, &method.class, &method.name, &signature, args)
    {
        if std::env::var_os("ARUNIKA_TRACE").is_some() {
            eprintln!("[trace]   -> {result:?}");
        }
        return match result {
            Ok(value) => Ok(FrameExit::Return(Some(value))),
            Err(message) => {
                if message == crate::MAIN_LOOP_IDLE {
                    return Err(DexError::MainLoopIdle);
                }
                if let Some(exception) = crate::NativeException::decode(&message) {
                    let text = heap.alloc(Object::String(exception.message));
                    let mut fields = HashMap::new();
                    fields.insert(0, text);
                    let object = heap.alloc(Object::Instance {
                        type_name: exception.class,
                        fields,
                    });
                    return Ok(FrameExit::Throw(object));
                }
                Err(DexError::NativeFailure {
                    class: method.class.clone(),
                    method: method.name.clone(),
                    message,
                })
            }
        };
    }

    Err(DexError::NoCode {
        class: method.class.clone(),
        method: method.name.clone(),
        signature,
    })
}
