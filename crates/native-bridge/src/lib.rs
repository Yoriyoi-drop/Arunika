//! Jembatan native (JNI) untuk fase 3 MVP.
//!
//! Crate ini menyediakan model domain JNI (JavaVM, JNIEnv, pencarian class/
//! method/field, pemanggilan method) tanpa memuat library ELF native ARM
//! yang masih belum ada. Peringatan DESAIN.md: "DEX berjalan bukan berarti
//! ELF ARM berjalan. Translation ISA adalah pekerjaan terpisah."
//!
//! Semua operasi di sini adalah model dalam proses dan dapat diuji secara
//! terisolasi. Integrasi JNI nyata dengan library native dilakukan melalui
//! implementasi platform native-bridge di fase 3/4.

use serde::Serialize;
use std::collections::HashMap;
use thiserror::Error;

/// Kesalahan jembatan native.
#[derive(Debug, Error)]
pub enum JniError {
    #[error("class tidak ditemukan: {0}")]
    ClassNotFound(String),
    #[error("method tidak ditemukan: {0}#{1}")]
    MethodNotFound(String, String),
    #[error("field tidak ditemukan: {0}#{1}")]
    FieldNotFound(String, String),
    #[error("jenis tipe tidak valid: {0}")]
    InvalidType(String),
    #[error("objek null tidak dapat dipakai sebagai pemanggil")]
    NullReceiver,
}

/// Hasil operasi JNI.
pub type JniResult<T> = Result<T, JniError>;

/// Tipe primitif dan referensi JNI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum JniType {
    Z,    // boolean
    B,    // byte
    C,    // char
    S,    // short
    I,    // int
    J,    // long
    F,    // float
    D,    // double
    V,    // void
    L,    // object (diikuti nama class)
    SArr, // array
}

impl JniType {
    /// Apakah tipe ini merepresentasikan referensi objek.
    pub fn is_reference(self) -> bool {
        matches!(self, JniType::L | JniType::SArr)
    }
}

/// Deskriptor tipe JNI, misalnya `I`, `Ljava/lang/String;`, `[I`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JniDescriptor(pub String);

impl JniDescriptor {
    pub fn parse(s: &str) -> JniResult<Self> {
        if s.is_empty() {
            return Err(JniError::InvalidType("deskriptor kosong".into()));
        }
        Ok(Self(s.to_owned()))
    }

    /// Tipe utama dari deskriptor.
    pub fn tipe_utama(&self) -> JniType {
        let b = self.0.as_bytes()[0];
        match b as char {
            'Z' => JniType::Z,
            'B' => JniType::B,
            'C' => JniType::C,
            'S' => JniType::S,
            'I' => JniType::I,
            'J' => JniType::J,
            'F' => JniType::F,
            'D' => JniType::D,
            'V' => JniType::V,
            'L' => JniType::L,
            '[' => JniType::SArr,
            _ => JniType::I,
        }
    }

    /// Nama class bila deskriptor bertipe `L...;`.
    pub fn nama_class(&self) -> Option<&str> {
        self.0.strip_prefix('L').and_then(|s| s.strip_suffix(';'))
    }
}

/// Satu method native terdaftar.
#[derive(Debug, Clone, Serialize)]
pub struct JniMethod {
    pub nama: String,
    pub deskriptor: JniDescriptor,
    pub statis: bool,
}

/// Satu field native terdaftar.
#[derive(Debug, Clone, Serialize)]
pub struct JniField {
    pub nama: String,
    pub deskriptor: JniDescriptor,
    pub statis: bool,
}

/// Satu class native terdaftar.
#[derive(Debug, Clone, Serialize, Default)]
pub struct JniClass {
    pub nama: String,
    pub methods: HashMap<String, JniMethod>,
    pub fields: HashMap<String, JniField>,
}

impl JniClass {
    pub fn baru(nama: impl Into<String>) -> Self {
        Self {
            nama: nama.into(),
            ..Default::default()
        }
    }

    pub fn tambah_method(mut self, method: JniMethod) -> Self {
        self.methods.insert(method.nama.clone(), method);
        self
    }

    pub fn tambah_field(mut self, field: JniField) -> Self {
        self.fields.insert(field.nama.clone(), field);
        self
    }
}

/// Nilai yang dapat dikirim melalui pemanggilan method native.
#[derive(Debug, Clone, PartialEq)]
pub enum JniValue {
    Void,
    Bool(bool),
    Byte(i8),
    Char(u16),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Object(u64),
}

/// Kontrak implementasi method native yang dapat dipanggil melalui JNI.
pub trait NativeMethod: Send + Sync {
    fn call(&self, args: &[JniValue]) -> JniResult<JniValue>;
}

/// Pembungkus closure sebagai implementasi native.
pub struct FnNativeMethod<F>
where
    F: Fn(&[JniValue]) -> JniResult<JniValue> + Send + Sync,
{
    f: F,
}

impl<F> NativeMethod for FnNativeMethod<F>
where
    F: Fn(&[JniValue]) -> JniResult<JniValue> + Send + Sync,
{
    fn call(&self, args: &[JniValue]) -> JniResult<JniValue> {
        (self.f)(args)
    }
}

/// Lingkungan JNI untuk satu thread.
#[derive(Clone, Default)]
pub struct JniEnv {
    classes: HashMap<String, JniClass>,
    next_object_id: u64,
    /// Objek instance yang diciptakan melalui JNI (id -> class).
    objek: HashMap<u64, String>,
    /// Implementasi method native yang didaftarkan runtime.
    native_methods: HashMap<(String, String), std::sync::Arc<dyn NativeMethod>>,
}

impl JniEnv {
    pub fn baru() -> Self {
        Self::default()
    }

    /// Daftarkan class native ke lingkungan.
    pub fn daftarkan_class(&mut self, class: JniClass) {
        self.classes.insert(class.nama.clone(), class);
    }

    /// Cari class berdasarkan nama (deskriptor atau nama lengkap).
    pub fn cari_class(&self, nama: &str) -> JniResult<&JniClass> {
        self.classes
            .get(nama)
            .ok_or_else(|| JniError::ClassNotFound(nama.to_owned()))
    }

    /// Dapatkan ID method dari class.
    pub fn dapatkan_method_id(&self, class_nama: &str, method_nama: &str) -> JniResult<&JniMethod> {
        let kelas = self.cari_class(class_nama)?;
        kelas
            .methods
            .get(method_nama)
            .ok_or_else(|| JniError::MethodNotFound(class_nama.to_owned(), method_nama.to_owned()))
    }

    /// Dapatkan ID field dari class.
    pub fn dapatkan_field_id(&self, class_nama: &str, field_nama: &str) -> JniResult<&JniField> {
        let kelas = self.cari_class(class_nama)?;
        kelas
            .fields
            .get(field_nama)
            .ok_or_else(|| JniError::FieldNotFound(class_nama.to_owned(), field_nama.to_owned()))
    }

    /// Buat instance objek baru dari class.
    pub fn buat_objek(&mut self, class_nama: &str) -> JniResult<u64> {
        self.cari_class(class_nama)?; // validasi class ada
        let id = self.next_object_id;
        self.next_object_id += 1;
        self.objek.insert(id, class_nama.to_owned());
        Ok(id)
    }

    /// Panggil method native (model: tidak ada implementasi sebenarnya,
    /// hanya mencatat pemanggilan dan mengembalikan placeholder).
    pub fn panggil_method(
        &mut self,
        objek_id: Option<u64>,
        class_nama: &str,
        method_nama: &str,
    ) -> JniResult<u64> {
        self.dapatkan_method_id(class_nama, method_nama)?;
        if let Some(id) = objek_id {
            self.objek.get(&id).ok_or(JniError::NullReceiver)?;
        }
        // Placeholder: native method asli belum diimplementasikan, kembalikan ID objek baru.
        let id = self.next_object_id;
        self.next_object_id += 1;
        self.objek.insert(id, class_nama.to_owned());
        Ok(id)
    }

    /// Daftarkan implementasi method native.
    pub fn daftarkan_native<F>(
        &mut self,
        class_nama: impl Into<String>,
        method_nama: impl Into<String>,
        f: F,
    ) where
        F: Fn(&[JniValue]) -> JniResult<JniValue> + Send + Sync + 'static,
    {
        self.native_methods.insert(
            (class_nama.into(), method_nama.into()),
            std::sync::Arc::new(FnNativeMethod { f }),
        );
    }

    /// Apakah method native memiliki implementasi.
    pub fn punya_native(&self, class_nama: &str, method_nama: &str) -> bool {
        self.native_methods
            .contains_key(&(class_nama.to_owned(), method_nama.to_owned()))
    }

    /// Panggil implementasi method native yang terdaftar.
    pub fn panggil_native(
        &self,
        class_nama: &str,
        method_nama: &str,
        args: &[JniValue],
    ) -> JniResult<JniValue> {
        let imp = self
            .native_methods
            .get(&(class_nama.to_owned(), method_nama.to_owned()))
            .ok_or_else(|| {
                JniError::MethodNotFound(class_nama.to_owned(), method_nama.to_owned())
            })?;
        imp.call(args)
    }

    /// Jumlah method native yang terdaftar.
    pub fn jumlah_native(&self) -> usize {
        self.native_methods.len()
    }

    /// Jumlah class terdaftar.
    pub fn jumlah_class(&self) -> usize {
        self.classes.len()
    }

    /// Jumlah objek yang diciptakan.
    pub fn jumlah_objek(&self) -> usize {
        self.objek.len()
    }
}

/// Mesin virtual Java (model): menyediakan JNIEnv per thread.
#[derive(Clone, Default)]
pub struct JavaVm {
    env: JniEnv,
}

impl JavaVm {
    pub fn baru() -> Self {
        Self::default()
    }

    /// Ambil referensi JNIEnv (model satu thread: env bersama).
    pub fn dapatkan_env(&mut self) -> &mut JniEnv {
        &mut self.env
    }
}

impl std::fmt::Debug for JavaVm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JavaVm")
            .field("classes", &self.env.jumlah_class())
            .field("objects", &self.env.jumlah_objek())
            .field("native_methods", &self.env.jumlah_native())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_awal_kosong() {
        let env = JniEnv::baru();
        assert_eq!(env.jumlah_class(), 0);
        assert_eq!(env.jumlah_objek(), 0);
    }

    #[test]
    fn daftarkan_dan_cari_class() {
        let mut env = JniEnv::baru();
        let kelas = JniClass::baru("Lcom/contoh/Foo;");
        env.daftarkan_class(kelas);
        assert_eq!(env.jumlah_class(), 1);
        assert!(env.cari_class("Lcom/contoh/Foo;").is_ok());
        assert!(env.cari_class("Lcom/contoh/Bar;").is_err());
    }

    #[test]
    fn method_dan_field_ditemukan() {
        let mut env = JniEnv::baru();
        let kelas = JniClass::baru("Lcom/contoh/Foo;")
            .tambah_method(JniMethod {
                nama: "hitung".to_string(),
                deskriptor: JniDescriptor::parse("(II)I").unwrap(),
                statis: true,
            })
            .tambah_field(JniField {
                nama: "nilai".to_string(),
                deskriptor: JniDescriptor::parse("I").unwrap(),
                statis: false,
            });
        env.daftarkan_class(kelas);

        let m = env
            .dapatkan_method_id("Lcom/contoh/Foo;", "hitung")
            .unwrap();
        assert_eq!(m.nama, "hitung");
        assert!(m.statis);

        let f = env.dapatkan_field_id("Lcom/contoh/Foo;", "nilai").unwrap();
        assert_eq!(f.nama, "nilai");
        assert!(!f.statis);
    }

    #[test]
    fn class_tidak_ditemukan_dan_method_tidak_ditemukan() {
        let mut env = JniEnv::baru();
        env.daftarkan_class(JniClass::baru("Lcom/contoh/Foo;"));
        let err = env.cari_class("Lcom/contoh/Bar;").unwrap_err();
        assert!(matches!(err, JniError::ClassNotFound { .. }));
        let err = env
            .dapatkan_method_id("Lcom/contoh/Foo;", "tidakAda")
            .unwrap_err();
        assert!(matches!(err, JniError::MethodNotFound { .. }));
    }

    #[test]
    fn buat_objek_dan_panggil_method() {
        let mut env = JniEnv::baru();
        env.daftarkan_class(JniClass::baru("Lcom/contoh/Foo;").tambah_method(JniMethod {
            nama: "hitung".to_string(),
            deskriptor: JniDescriptor::parse("()V").unwrap(),
            statis: false,
        }));
        let id = env.buat_objek("Lcom/contoh/Foo;").unwrap();
        assert_eq!(id, 0);
        assert_eq!(env.jumlah_objek(), 1);

        let _ = env
            .panggil_method(Some(id), "Lcom/contoh/Foo;", "hitung")
            .unwrap();
        assert_eq!(env.jumlah_objek(), 2); // placeholder membuat objek baru
    }

    #[test]
    fn panggil_method_objek_null_ditolak() {
        let mut env = JniEnv::baru();
        env.daftarkan_class(JniClass::baru("Lcom/contoh/Foo;").tambah_method(JniMethod {
            nama: "hitung".to_string(),
            deskriptor: JniDescriptor::parse("()V").unwrap(),
            statis: false,
        }));
        let err = env
            .panggil_method(Some(999), "Lcom/contoh/Foo;", "hitung")
            .unwrap_err();
        assert!(matches!(err, JniError::NullReceiver));
    }

    #[test]
    fn deskriptor_tipe_utama() {
        assert_eq!(JniDescriptor::parse("I").unwrap().tipe_utama(), JniType::I);
        assert_eq!(
            JniDescriptor::parse("Ljava/lang/String;")
                .unwrap()
                .tipe_utama(),
            JniType::L
        );
        assert_eq!(
            JniDescriptor::parse("[I").unwrap().tipe_utama(),
            JniType::SArr
        );
        assert_eq!(
            JniDescriptor::parse("Ljava/lang/String;")
                .unwrap()
                .nama_class(),
            Some("java/lang/String")
        );
    }

    #[test]
    fn deskriptor_kosong_ditolak() {
        assert!(JniDescriptor::parse("").is_err());
    }

    #[test]
    fn jvm_dapatkan_env() {
        let mut jvm = JavaVm::baru();
        let env = jvm.dapatkan_env();
        env.daftarkan_class(JniClass::baru("Lcom/contoh/Foo;"));
        assert_eq!(jvm.dapatkan_env().jumlah_class(), 1);
    }

    #[test]
    fn jni_error_tampilkan() {
        let e = JniError::ClassNotFound("Lcom/contoh/Bar;".into());
        assert_eq!(e.to_string(), "class tidak ditemukan: Lcom/contoh/Bar;");
        let e2 = JniError::MethodNotFound("Foo".into(), "bar".into());
        assert_eq!(e2.to_string(), "method tidak ditemukan: Foo#bar");
        let e3 = JniError::FieldNotFound("Foo".into(), "nilai".into());
        assert_eq!(e3.to_string(), "field tidak ditemukan: Foo#nilai");
        let e4 = JniError::NullReceiver;
        assert_eq!(
            e4.to_string(),
            "objek null tidak dapat dipakai sebagai pemanggil"
        );
    }

    #[test]
    fn class_serialisasi_json() {
        let kelas = JniClass::baru("Lcom/contoh/Foo;");
        let json = serde_json::to_string(&kelas).unwrap();
        assert!(json.contains("Lcom/contoh/Foo;"));
    }

    #[test]
    fn native_method_terdaftar_dan_dipanggil() {
        let mut env = JniEnv::baru();
        env.daftarkan_class(JniClass::baru("Lcom/contoh/Foo;").tambah_method(JniMethod {
            nama: "hitung".to_string(),
            deskriptor: JniDescriptor::parse("(II)I").unwrap(),
            statis: true,
        }));

        env.daftarkan_native("Lcom/contoh/Foo;", "hitung", |args| {
            let a = match args.first() {
                Some(JniValue::Int(v)) => *v,
                _ => return Err(JniError::InvalidType("argumen pertama bukan int".into())),
            };
            let b = match args.get(1) {
                Some(JniValue::Int(v)) => *v,
                _ => return Err(JniError::InvalidType("argumen kedua bukan int".into())),
            };
            Ok(JniValue::Int(a + b))
        });

        assert!(env.punya_native("Lcom/contoh/Foo;", "hitung"));
        let result = env
            .panggil_native(
                "Lcom/contoh/Foo;",
                "hitung",
                &[JniValue::Int(2), JniValue::Int(3)],
            )
            .expect("native harus terpanggil");
        assert_eq!(result, JniValue::Int(5));
    }

    #[test]
    fn native_method_tidak_terdaftar_ditolak() {
        let env = JniEnv::baru();
        let err = env
            .panggil_native("Lcom/contoh/Foo;", "hitung", &[])
            .unwrap_err();
        assert!(matches!(err, JniError::MethodNotFound { .. }));
    }
}
