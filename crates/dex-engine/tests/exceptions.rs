//! Uji semantik eksepsi DEX (fase 1.3).
//!
//! Cakupan: `throw`, try/catch bertipe, catch-all, pencocokan lewat
//! superclass, dan perambatan eksepsi lintas frame method.

use arunika_dex_engine::{DexEngine, DexError, Value, parse_bytes};
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, CatchSpec, ClassSpec, DexBuild, MethodSpec, build_dex_with_symbols,
    insn,
};

const EXCEPTION: &str = "Ljava/lang/Exception;";
const RUNTIME_EXCEPTION: &str = "Ljava/lang/RuntimeException;";

fn engine(bytes: &[u8]) -> DexEngine {
    let dex = parse_bytes(bytes, "eksepsi.dex").expect("DEX fixture harus terparse");
    DexEngine::new(dex)
}

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

#[test]
fn eksepsi_tertangkap_mengembalikan_nilai_handler() {
    // static int jalankan() {
    //     try { throw new Exception(); } catch (Exception e) { return 7; }
    // }
    let build = |type_index: u16| {
        let method = static_method(
            "jalankan",
            &[],
            "I",
            2,
            0,
            [
                insn::new_instance(0, type_index), // pc 0..1
                insn::throw_(0),                   // pc 2
                insn::move_exception(0),           // pc 3
                insn::const_16(1, 7),              // pc 4..5
                insn::return_(1),                  // pc 6
            ]
            .concat(),
        )
        .try_catch(0, 3, CatchSpec::typed(EXCEPTION, 3));

        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method)
        ])
    };

    let type_index = build(0).type_index(EXCEPTION);
    let mut engine = engine(&build(type_index).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("eksepsi harus tertangkap");

    assert_eq!(outcome.return_value, Some(Value::Int(7)));
}

#[test]
fn eksepsi_tanpa_handler_menjadi_error_eksplisit() {
    let build = |type_index: u16| {
        let method = static_method(
            "meledak",
            &[],
            "V",
            1,
            0,
            [insn::new_instance(0, type_index), insn::throw_(0)].concat(),
        );
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method),
            ClassSpec::new(EXCEPTION, Some("Ljava/lang/Throwable;")),
        ])
    };

    let type_index = build(0).type_index(EXCEPTION);
    let mut engine = engine(&build(type_index).bytes);
    let error = engine
        .execute_method("LContoh;", "meledak")
        .expect_err("eksepsi tanpa handler harus gagal");

    match error {
        DexError::UncaughtException { type_name } => assert_eq!(type_name, EXCEPTION),
        other => panic!("error tak terduga: {other:?}"),
    }
}

#[test]
fn catch_all_menangkap_tipe_tanpa_pasangan() {
    let build = |type_index: u16| {
        let method = static_method(
            "jalankan",
            &[],
            "I",
            2,
            0,
            [
                insn::new_instance(0, type_index), // pc 0..1
                insn::throw_(0),                   // pc 2
                insn::move_exception(1),           // pc 3
                insn::const_16(0, 11),             // pc 4..5
                insn::return_(0),                  // pc 6
            ]
            .concat(),
        )
        .try_catch(0, 3, CatchSpec::catch_all(3));

        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method),
            ClassSpec::new(RUNTIME_EXCEPTION, Some(EXCEPTION)),
        ])
    };

    let type_index = build(0).type_index(RUNTIME_EXCEPTION);
    let mut engine = engine(&build(type_index).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("catch-all harus menangkap");

    assert_eq!(outcome.return_value, Some(Value::Int(11)));
}

#[test]
fn catch_pada_superclass_menangkap_subclass() {
    // LAnak; extends LIbu; extends Ljava/lang/Object;
    // throw new LAnak() ditangkap oleh catch (LIbu; e).
    let build = |type_index: u16| {
        let method = static_method(
            "jalankan",
            &[],
            "I",
            2,
            0,
            [
                insn::new_instance(0, type_index), // pc 0..1
                insn::throw_(0),                   // pc 2
                insn::move_exception(0),           // pc 3
                insn::const_16(1, 5),              // pc 4..5
                insn::return_(1),                  // pc 6
            ]
            .concat(),
        )
        .try_catch(0, 3, CatchSpec::typed("LIbu;", 3));

        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method),
            ClassSpec::new("LAnak;", Some("LIbu;")),
            ClassSpec::new("LIbu;", Some("Ljava/lang/Object;")),
        ])
    };

    let type_index = build(0).type_index("LAnak;");
    let mut engine = engine(&build(type_index).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("catch pada superclass harus berlaku");

    assert_eq!(outcome.return_value, Some(Value::Int(5)));
}

#[test]
fn melempar_nilai_bukan_referensi_ditolak() {
    let bytes = build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(
        static_method(
            "salah",
            &[],
            "V",
            1,
            0,
            [insn::const_4(0, 1), insn::throw_(0)].concat(),
        ),
    )])
    .bytes;

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "salah")
        .expect_err("throw atas int harus ditolak");

    match error {
        DexError::ThrowNonReference { pc } => assert_eq!(pc, 1),
        other => panic!("error tak terduga: {other:?}"),
    }
}

#[test]
fn eksepsi_dari_method_lain_ditangkap_pemanggil() {
    // LPemanggil.jalankan(): try { LPeledak.boom(); return 1; } catch (RuntimeException e) { return 9; }
    let build = |type_index: u16, boom: u16| -> DexBuild {
        let caller = static_method(
            "jalankan",
            &[],
            "I",
            2,
            0,
            [
                insn::invoke_static(boom, &[]), // pc 0..2
                insn::const_16(0, 1),           // pc 3..4
                insn::return_(0),               // pc 5
                insn::move_exception(0),        // pc 6
                insn::const_16(1, 9),           // pc 7..8
                insn::return_(1),               // pc 9
            ]
            .concat(),
        )
        .try_catch(0, 3, CatchSpec::typed(RUNTIME_EXCEPTION, 6));

        let thrower = static_method(
            "boom",
            &[],
            "V",
            1,
            0,
            [insn::new_instance(0, type_index), insn::throw_(0)].concat(),
        );

        build_dex_with_symbols(&[
            ClassSpec::new("LPemanggil;", Some("Ljava/lang/Object;")).with_method(caller),
            ClassSpec::new("LPeledak;", Some("Ljava/lang/Object;")).with_method(thrower),
        ])
    };

    let probe = build(0, 0);
    let type_index = probe.type_index(RUNTIME_EXCEPTION);
    let boom = probe.method_index("LPeledak;", "boom");

    let mut engine = engine(&build(type_index, boom).bytes);
    let outcome = engine
        .execute_method("LPemanggil;", "jalankan")
        .expect("pemanggil harus menangkap eksepsi callee");

    assert_eq!(outcome.return_value, Some(Value::Int(9)));
}
/// Resolver uji yang selalu melempar FileNotFoundException.
#[derive(Debug)]
struct FileHilang;

impl arunika_dex_engine::NativeResolver for FileHilang {
    fn invoke_static(
        &self,
        class_descriptor: &str,
        method_name: &str,
        _args: &[Value],
    ) -> Option<Result<Value, String>> {
        if class_descriptor == "Ljava/io/File;" && method_name == "buka" {
            return Some(Err(arunika_dex_engine::NativeException::new(
                "Ljava/io/FileNotFoundException;",
                "hilang",
            )
            .encode()));
        }
        None
    }
}

#[test]
fn eksepsi_native_ditangkap_lewat_superclass_framework() {
    // try { LFile.buka(); return 1; } catch (IOException e) { return 7; }
    let build = |open: u16| -> DexBuild {
        let file = ClassSpec::new("Ljava/io/File;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("buka", ACC_PUBLIC | ACC_STATIC, &[], "V"));
        let caller = static_method(
            "baca",
            &[],
            "I",
            2,
            0,
            [
                insn::invoke_static(open, &[]),
                insn::const_16(0, 1),
                insn::return_(0),
                insn::move_exception(0),
                insn::const_16(1, 7),
                insn::return_(1),
            ]
            .concat(),
        )
        .try_catch(0, 3, CatchSpec::typed("Ljava/io/IOException;", 6));

        build_dex_with_symbols(&[
            file,
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(caller),
        ])
    };

    let probe = build(0);
    let open = probe.method_index("Ljava/io/File;", "buka");
    let bytes = build(open).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(FileHilang));

    let outcome = engine
        .execute_method("LContoh;", "baca")
        .expect("eksepsi native harus ditangkap");
    assert_eq!(outcome.return_value, Some(Value::Int(7)));
}

#[test]
fn catch_all_dapat_menjalankan_finally_lalu_melempar_ulang() {
    // Pola finally Java: tangkap eksepsi apa pun (catch-all), lalu lempar ulang.
    let build = |type_index: u16| {
        let method = static_method(
            "jalankan",
            &[],
            "V",
            2,
            0,
            [
                insn::new_instance(0, type_index), // pc 0..1
                insn::throw_(0),                   // pc 2
                insn::move_exception(0),           // pc 3
                insn::throw_(0),                   // pc 4
            ]
            .concat(),
        )
        .try_catch(0, 3, CatchSpec::catch_all(3));

        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method),
            ClassSpec::new(EXCEPTION, Some("Ljava/lang/Throwable;")),
        ])
    };

    let type_index = build(0).type_index(EXCEPTION);
    let mut engine = engine(&build(type_index).bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("eksepsi harus tetap merambat");

    match error {
        DexError::UncaughtException { type_name } => assert_eq!(type_name, EXCEPTION),
        other => panic!("error tak terduga: {other:?}"),
    }
}
