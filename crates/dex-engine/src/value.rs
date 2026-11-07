//! Model nilai runtime DEX.
//!
//! Register DEX menyimpan satu word 32-bit. Nilai numerik disimpan sebagai
//! bit mentah dan ditafsirkan oleh opcode: `Value::Int` untuk word 32-bit
//! (int maupun float), `Value::Wide` untuk pasangan word 64-bit (long maupun
//! double), dan `Value::Ref` untuk referensi objek.
//!
//! Long dan double menempati dua register DEX, tetapi di sini nilainya
//! disimpan utuh pada register pertama. Ini aman karena DEX selalu merujuk
//! pasangan lewat register pertama; register kedua tidak pernah dibaca
//! sebagai nilai tersendiri.

use serde::Serialize;

/// Nilai yang dapat berada di register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Value {
    /// Word 32-bit: int atau bit `f32`.
    Int(i32),
    /// Pasangan word 64-bit: long atau bit `f64`.
    Wide(u64),
    /// Referensi ke objek pada [`Heap`].
    Ref(u32),
}

impl Value {
    pub fn as_int(self) -> Option<i32> {
        match self {
            Value::Int(value) => Some(value),
            Value::Wide(_) | Value::Ref(_) => None,
        }
    }

    pub fn as_ref(self) -> Option<u32> {
        match self {
            Value::Ref(index) => Some(index),
            Value::Int(_) | Value::Wide(_) => None,
        }
    }

    /// Bit mentah 64-bit untuk long maupun double.
    pub fn as_wide(self) -> Option<u64> {
        match self {
            Value::Wide(value) => Some(value),
            Value::Int(_) | Value::Ref(_) => None,
        }
    }

    pub fn as_long(self) -> Option<i64> {
        self.as_wide().map(|bits| bits as i64)
    }

    pub fn as_float(self) -> Option<f32> {
        self.as_int().map(|bits| f32::from_bits(bits as u32))
    }

    pub fn as_double(self) -> Option<f64> {
        self.as_wide().map(f64::from_bits)
    }

    pub fn from_float(value: f32) -> Self {
        Value::Int(value.to_bits() as i32)
    }

    pub fn from_double(value: f64) -> Self {
        Value::Wide(value.to_bits())
    }

    pub fn from_long(value: i64) -> Self {
        Value::Wide(value as u64)
    }
}

/// Objek yang dialokasikan runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Object {
    /// String Java/DEX. Disimpan sebagai UTF-8 Rust.
    String(String),
    /// Instance class. Field disimpan per indeks `field_ids`, sehingga
    /// pewarisan tidak butuh pemetaan tambahan.
    Instance {
        type_name: String,
        fields: std::collections::HashMap<u32, Value>,
    },
    /// Array dengan panjang tetap. Tipe elemen disimpan untuk pesan error.
    Array {
        element_type: String,
        elements: Vec<Value>,
    },
    /// Objek `java.lang.Class` hasil `const-class`.
    Class { descriptor: String },
    /// Koleksi `java.util` yang disediakan runtime (ArrayList/LinkedList).
    List {
        type_name: String,
        elements: Vec<Value>,
    },
    /// Nilai primitif yang di-box (mis. `Integer.valueOf`).
    Boxed { type_name: String, value: Value },
    /// Map `java.util` yang disediakan runtime (HashMap/LinkedHashMap).
    Map {
        type_name: String,
        entries: Vec<(Value, Value)>,
    },
    /// Method yang ditemukan lewat reflection.
    ///
    /// Disimpan sebagai deskripsi, bukan pointer ke code item, sehingga objek
    /// ini tetap sah setelah heap berubah.
    Method {
        class: String,
        name: String,
        parameters: Vec<String>,
    },
    /// Aliran byte yang disediakan runtime (`FileInputStream`/`FileOutputStream`).
    Stream {
        type_name: String,
        /// Nama berkas di direktori data aplikasi tempat aliran terikat.
        name: String,
        data: Vec<u8>,
        position: usize,
    },
    /// Buffer piksel RGBA offscreen (jendela/bitmap). Operasi gambar canvas
    /// merekam ke buffer ini; tidak ada grafis host yang terlibat.
    Bitmap {
        width: u32,
        height: u32,
        /// Piksel RGBA, panjang = width * height * 4.
        pixels: Vec<u8>,
    },
}

/// Heap sederhana dengan indeks stabil.
///
/// Tidak ada garbage collector: objek hidup selama engine hidup. Ini cukup
/// untuk proof of concept dan tidak boleh dianggap model memori produksi.
#[derive(Debug, Default, Clone)]
pub struct Heap {
    objects: Vec<Object>,
}

impl Heap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self, object: Object) -> Value {
        let index = self.objects.len() as u32;
        self.objects.push(object);
        Value::Ref(index)
    }

    pub fn get(&self, index: u32) -> Option<&Object> {
        self.objects.get(index as usize)
    }

    pub fn get_mut(&mut self, index: u32) -> Option<&mut Object> {
        self.objects.get_mut(index as usize)
    }

    pub fn string(&self, index: u32) -> Option<&str> {
        match self.get(index)? {
            Object::String(value) => Some(value),
            Object::Instance { .. }
            | Object::Array { .. }
            | Object::Class { .. }
            | Object::List { .. }
            | Object::Boxed { .. }
            | Object::Map { .. }
            | Object::Method { .. }
            | Object::Stream { .. }
            | Object::Bitmap { .. } => None,
        }
    }

    /// Panjang array pada referensi tertentu, bila objeknya array.
    pub fn array_len(&self, index: u32) -> Option<usize> {
        match self.get(index)? {
            Object::Array { elements, .. } => Some(elements.len()),
            Object::String(_)
            | Object::Instance { .. }
            | Object::Class { .. }
            | Object::List { .. }
            | Object::Boxed { .. }
            | Object::Map { .. }
            | Object::Method { .. }
            | Object::Stream { .. }
            | Object::Bitmap { .. } => None,
        }
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_memberi_indeks_stabil() {
        let mut heap = Heap::new();
        let first = heap.alloc(Object::String("halo".to_string()));
        let second = heap.alloc(Object::String("dunia".to_string()));

        assert_eq!(first, Value::Ref(0));
        assert_eq!(second, Value::Ref(1));
        assert_eq!(heap.string(0), Some("halo"));
        assert_eq!(heap.string(1), Some("dunia"));
        assert_eq!(heap.len(), 2);
    }

    #[test]
    fn nilai_int_dan_ref_tidak_tertukar() {
        assert_eq!(Value::Int(7).as_int(), Some(7));
        assert_eq!(Value::Int(7).as_ref(), None);
        assert_eq!(Value::Ref(3).as_ref(), Some(3));
        assert_eq!(Value::Ref(3).as_int(), None);
    }

    #[test]
    fn nilai_wide_menyimpan_bit_long_dan_double() {
        assert_eq!(Value::from_long(-5).as_long(), Some(-5));
        assert_eq!(Value::from_double(1.5).as_double(), Some(1.5));
        assert_eq!(Value::from_float(0.25).as_float(), Some(0.25));
        assert_eq!(Value::from_long(1).as_int(), None);
        assert_eq!(Value::Int(1).as_wide(), None);
    }
}
