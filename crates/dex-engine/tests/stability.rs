//! Uji stabilitas interpreter DEX subset (fase 1).
//!
//! Fokus: nilai wide (long/double), referensi null, batas array,
//! penanganan eksepsi, inisialisasi kelas (<clinit>), batas langkah
//! (MAX_STEPS), dan batas panjang array (MAX_ARRAY_LEN). Semua kasus
//! harus menghasilkan error eksplisit, tidak panic.

use arunika_dex_engine::{DexEngine, DexError, Value, parse_bytes};
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, CatchSpec, ClassSpec, MethodSpec, build_dex_with_symbols, insn,
};

fn static_method(
    name: &str,
    params: &[&str],
    ret: &str,
    registers: u16,
    ins: u16,
    body: Vec<u16>,
) -> MethodSpec {
    MethodSpec::new(name, ACC_PUBLIC | ACC_STATIC, params, ret)
        .registers(registers)
        .ins(ins)
        .body(body)
}

fn bytes_of(classes: &[ClassSpec]) -> Vec<u8> {
    build_dex_with_symbols(classes).bytes
}

fn engine(bytes: &[u8]) -> DexEngine {
    let dex = parse_bytes(bytes, "stabilitas.dex").expect("DEX fixture harus terparse");
    DexEngine::new(dex)
}

// ---------------------------------------------------------------------------
// Nilai wide (long / double)
// ---------------------------------------------------------------------------

#[test]
fn wide_values_long_arithmetika_benar() {
    // long a = 10000000000; long b = 3; return a / b;
    let bytes = bytes_of(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
            "bagi",
            &[],
            "J",
            4,
            0,
            [
                insn::const_wide(0, 10_000_000_000),
                insn::const_wide_16(2, 3),
                insn::div_long(0, 0, 2),
                insn::return_wide(0),
            ]
            .concat(),
        )),
    ]);

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "bagi")
        .expect("aritmetika long harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::from_long(3_333_333_333)));
}

#[test]
fn wide_values_move_wide_dan_return_wide() {
    // long a = 42; return a;
    let bytes = bytes_of(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
            "kembalikan",
            &[],
            "J",
            2,
            0,
            [insn::const_wide_16(0, 42), insn::return_wide(0)].concat(),
        )),
    ]);

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "kembalikan")
        .expect("move-wide/return-wide harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::from_long(42)));
}

// ---------------------------------------------------------------------------
// Referensi null
// ---------------------------------------------------------------------------

#[test]
fn null_reference_pada_iget_ditolak() {
    // Object o = null; o.field;
    let field = arunika_test_fixtures::dex::FieldSpec::new("field", "I", false);
    let bytes = bytes_of(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
        .with_field(field)
        .with_method(static_method(
            "jalankan",
            &[],
            "I",
            2,
            0,
            [insn::const_4(0, 0), insn::iget(1, 0, 0), insn::return_(1)].concat(),
        ))]);

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("iget pada null harus ditolak");
    match error {
        DexError::NullReference { .. } => {}
        other => panic!("error tak terduga: {other:?}"),
    }
}

#[test]
fn null_reference_pada_array_length_ditolak() {
    // Object[] a = null; a.length;
    let bytes = bytes_of(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
            "jalankan",
            &[],
            "I",
            2,
            0,
            [
                insn::const_4(0, 0),
                insn::array_length(1, 0),
                insn::return_(1),
            ]
            .concat(),
        )),
    ]);

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("array-length pada null harus ditolak");
    match error {
        DexError::NullReference { .. } => {}
        other => panic!("error tak terduga: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Batas array
// ---------------------------------------------------------------------------

#[test]
fn array_index_luar_batas_ditolak() {
    // int[] a = new int[3]; return a[5];
    let int_type = 0;
    let bytes = bytes_of(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
            "jalankan",
            &[],
            "I",
            3,
            0,
            [
                insn::const_4(0, 3),
                insn::new_array(0, 0, int_type),
                insn::const_4(1, 5),
                insn::aget(2, 0, 1),
                insn::return_(2),
            ]
            .concat(),
        )),
    ]);

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("akses indeks luar batas harus ditolak");
    match error {
        DexError::ArrayIndexOutOfBounds { .. } => {}
        other => panic!("error tak terduga: {other:?}"),
    }
}

#[test]
fn new_array_ukuran_negatif_ditolak() {
    // int size = -1; int[] a = new int[size];
    let int_type = 0;
    let bytes = bytes_of(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
            "jalankan",
            &[],
            "V",
            3,
            0,
            [
                insn::const_4(0, -1),
                insn::new_array(1, 0, int_type),
                insn::return_void(),
            ]
            .concat(),
        )),
    ]);

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("ukuran array negatif harus ditolak");
    match error {
        DexError::NegativeArraySize { .. } => {}
        other => panic!("error tak terduga: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// MAX_ARRAY_LEN
// ---------------------------------------------------------------------------

#[test]
fn new_array_melebihi_max_array_len_ditolak() {
    // int[] a = new int[MAX_ARRAY_LEN + 1];
    let int_type = 0;
    let bytes = bytes_of(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
            "jalankan",
            &[],
            "V",
            2,
            0,
            [
                insn::const_(0, 1048577), // MAX_ARRAY_LEN + 1
                insn::new_array(1, 0, int_type),
                insn::return_void(),
            ]
            .concat(),
        )),
    ]);

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("array melebihi MAX_ARRAY_LEN harus ditolak");
    match error {
        DexError::LimitExceeded { table: "array", .. } => {}
        other => panic!("error tak terduga: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Inisialisasi kelas (<clinit>)
// ---------------------------------------------------------------------------

#[test]
fn clinit_dijalankan_saat_class_pertama_kali_dipakai() {
    // static int counter = 5; (via <clinit>)
    // static int getCounter() { return counter; }
    let field = arunika_test_fixtures::dex::FieldSpec::new("counter", "I", true);
    let bytes = bytes_of(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
        .with_field(field)
        .with_method(static_method(
            "<clinit>",
            &[],
            "V",
            1,
            0,
            [insn::const_4(0, 5), insn::sput(0, 0), insn::return_void()].concat(),
        ))
        .with_method(static_method(
            "getCounter",
            &[],
            "I",
            1,
            0,
            [insn::sget(0, 0), insn::return_(0)].concat(),
        ))]);

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "getCounter")
        .expect("clinit harus dijalankan otomatis");
    assert_eq!(outcome.return_value, Some(Value::Int(5)));
}

#[test]
fn clinit_hanya_dijalankan_satu_kali() {
    // static int initCalls = 0;
    // static { initCalls++; }
    let field = arunika_test_fixtures::dex::FieldSpec::new("initCalls", "I", true);
    let bytes = bytes_of(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
        .with_field(field)
        .with_method(static_method(
            "<clinit>",
            &[],
            "V",
            2,
            0,
            [
                insn::sget(0, 0),
                insn::const_4(1, 1),
                insn::add_int(0, 0, 1),
                insn::sput(0, 0),
                insn::return_void(),
            ]
            .concat(),
        ))
        .with_method(static_method(
            "getCalls",
            &[],
            "I",
            1,
            0,
            [insn::sget(0, 0), insn::return_(0)].concat(),
        ))]);

    let mut engine = engine(&bytes);
    let _ = engine.execute_method("LContoh;", "getCalls");
    let _ = engine.execute_method("LContoh;", "getCalls");
    let outcome = engine
        .execute_method("LContoh;", "getCalls")
        .expect("clinit hanya sekali, initCalls tetap 1");
    assert_eq!(outcome.return_value, Some(Value::Int(1)));
}

// ---------------------------------------------------------------------------
// Batas langkah (MAX_STEPS)
// ---------------------------------------------------------------------------

#[test]
fn step_limit_dilaporkan_sebagai_error_eksplisit() {
    // while (true) {} -> looping tanpa akhir hingga MAX_STEPS
    let bytes = bytes_of(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
            "loop",
            &[],
            "V",
            1,
            0,
            [insn::goto(0)].concat(),
        )),
    ]);

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "loop")
        .expect_err("loop tanpa akhir harus terdeteksi lewat batas langkah");
    match error {
        DexError::ExecutionLimit { .. } => {}
        other => panic!("error tak terduga: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Eksepsi try/catch edge cases
// ---------------------------------------------------------------------------

#[test]
fn eksepsi_yang_tidak_tertangkap_menjadi_uncaught_exception() {
    // throw new RuntimeException(); (tanpa try/catch)
    let runtime_ex = 0;
    let bytes = bytes_of(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
            "jalankan",
            &[],
            "V",
            1,
            0,
            [
                insn::new_instance(0, runtime_ex),
                insn::throw_(0),
                insn::return_void(),
            ]
            .concat(),
        )),
    ]);

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("eksepsi yang tidak tertangkap harus ditolak");
    match error {
        DexError::UncaughtException { .. } => {}
        other => panic!("error tak terduga: {other:?}"),
    }
}

#[test]
fn catch_all_tangkap_semua_eksepsi() {
    // try { throw new Exception(); } catch (Throwable t) { return 9; }
    let runtime_ex = 0;
    let bytes = bytes_of(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(
            static_method(
                "jalankan",
                &[],
                "I",
                2,
                0,
                [
                    insn::new_instance(0, runtime_ex),
                    insn::throw_(0),
                    insn::move_exception(0),
                    insn::const_16(1, 9),
                    insn::return_(1),
                ]
                .concat(),
            )
            .try_catch(0, 3, CatchSpec::catch_all(3)),
        ),
    ]);

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("catch-all harus menangkap eksepsi");
    assert_eq!(outcome.return_value, Some(Value::Int(9)));
}
