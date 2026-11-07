//! Uji eksekusi bytecode DEX subset (fase 1).

use arunika_dex_engine::{DexEngine, DexError, NativeResolver, Object, Value, parse_bytes};
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, ClassSpec, MethodSpec, build_dex_with_symbols, insn,
};

fn engine(bytes: &[u8]) -> DexEngine {
    let dex = parse_bytes(bytes, "uji.dex").expect("DEX fixture harus terparse");
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
fn mengembalikan_konstanta() {
    let bytes = build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(
        static_method(
            "jawab",
            &[],
            "I",
            1,
            0,
            [insn::const_16(0, 42), insn::return_(0)].concat(),
        ),
    )])
    .bytes;

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "jawab")
        .expect("eksekusi harus berhasil");

    assert!(outcome.executed);
    assert_eq!(outcome.return_value, Some(Value::Int(42)));
}

#[test]
fn menjumlahkan_dua_parameter() {
    let bytes = build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(
        static_method(
            "tambah",
            &["I", "I"],
            "I",
            2,
            2,
            [insn::add_int(0, 0, 1), insn::return_(0)].concat(),
        ),
    )])
    .bytes;

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method_with_args("LContoh;", "tambah", &[7, 35])
        .expect("eksekusi harus berhasil");

    assert_eq!(outcome.return_value, Some(Value::Int(42)));
}

#[test]
fn percabangan_menghitung_nilai_absolut() {
    // int abs(int x) { if (x >= 0) return x; return 0 - x; }
    // v1 = x (parameter), v0 = register sementara.
    let body = [
        vec![0x3b | (1 << 8), 6], // if-ltz v1, +6 -> pc 6
        insn::const_4(0, 0),
        insn::sub_int(0, 0, 1),
        insn::return_(0),
        insn::return_(1),
    ]
    .concat();

    let bytes =
        build_dex_with_symbols(
            &[ClassSpec::new("LContoh;", None).with_method(static_method(
                "abs",
                &["I"],
                "I",
                2,
                1,
                body,
            ))],
        )
        .bytes;

    let mut engine = engine(&bytes);
    assert_eq!(
        engine
            .execute_method_with_args("LContoh;", "abs", &[-9])
            .expect("eksekusi harus berhasil")
            .return_value,
        Some(Value::Int(9))
    );
    assert_eq!(
        engine
            .execute_method_with_args("LContoh;", "abs", &[7])
            .expect("eksekusi harus berhasil")
            .return_value,
        Some(Value::Int(7))
    );
}

#[test]
fn perulangan_menghitung_jumlah_1_sampai_n() {
    // int sum(int n) { int s = 0; for (int i = 1; i <= n; i++) s += i; return s; }
    let body = [
        insn::const_4(0, 0),
        insn::const_4(1, 1),
        insn::const_4(2, 1),
        vec![0x36 | (1 << 8) | (3 << 12), 7], // if-gt v1, v3, +7 -> pc 10
        insn::add_int(0, 0, 1),
        insn::add_int(1, 1, 2),
        insn::goto(-6),
        insn::return_(0),
    ]
    .concat();

    let bytes =
        build_dex_with_symbols(
            &[ClassSpec::new("LContoh;", None).with_method(static_method(
                "sum",
                &["I"],
                "I",
                4,
                1,
                body,
            ))],
        )
        .bytes;

    let mut engine = engine(&bytes);
    assert_eq!(
        engine
            .execute_method_with_args("LContoh;", "sum", &[10])
            .expect("eksekusi harus berhasil")
            .return_value,
        Some(Value::Int(55))
    );
}

#[test]
fn memanggil_method_statis_lain_dan_memakai_move_result() {
    let build = |helper_index: u16| {
        let helper = static_method(
            "helper",
            &["I"],
            "I",
            2,
            1,
            [
                insn::const_4(0, 1),
                insn::add_int(0, 0, 1),
                insn::return_(0),
            ]
            .concat(),
        );
        let caller = static_method(
            "caller",
            &[],
            "I",
            1,
            0,
            [
                insn::const_4(0, 5),
                insn::invoke_static(helper_index, &[0]),
                insn::move_result(0),
                insn::return_(0),
            ]
            .concat(),
        );
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", None)
            .with_method(helper)
            .with_method(caller)])
    };

    let probe = build(0);
    let helper_index = probe.method_index("LContoh;", "helper");
    let bytes = build(helper_index).bytes;

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "caller")
        .expect("eksekusi harus berhasil");

    assert_eq!(outcome.return_value, Some(Value::Int(6)));
}

#[test]
fn rekursi_tanpa_batas_ditolak() {
    let build = |self_index: u16| {
        let method = static_method(
            "rekursif",
            &[],
            "V",
            0,
            0,
            [insn::invoke_static(self_index, &[]), insn::return_void()].concat(),
        );
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(method)])
    };

    let probe = build(0);
    let index = probe.method_index("LContoh;", "rekursif");
    let bytes = build(index).bytes;

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "rekursif")
        .expect_err("rekursi tanpa henti harus ditolak");
    assert!(
        matches!(error, DexError::CallDepthExceeded { .. }),
        "error: {error:?}"
    );
}

#[test]
fn perulangan_tanpa_henti_kena_batas_langkah() {
    // Loop dua arah tanpa offset nol: pc0 -> pc2 -> pc0.
    let body = [insn::goto(2), insn::nop(), insn::goto(-2)].concat();
    let bytes =
        build_dex_with_symbols(
            &[ClassSpec::new("LContoh;", None).with_method(static_method(
                "hang",
                &[],
                "V",
                0,
                0,
                body,
            ))],
        )
        .bytes;

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "hang")
        .expect_err("perulangan tanpa henti harus ditolak");
    assert!(
        matches!(error, DexError::ExecutionLimit { .. }),
        "error: {error:?}"
    );
}

#[test]
fn batas_langkah_melaporkan_method_terdalam() {
    // `pemanggil` memanggil `hang`, yang berputar tanpa henti. Galat harus
    // menunjuk method yang benar-benar berputar, bukan pemanggilnya.
    let build = |hang_index: u16| {
        let pemanggil = static_method(
            "pemanggil",
            &[],
            "V",
            0,
            0,
            [insn::invoke_static(hang_index, &[]), insn::return_void()].concat(),
        );
        let hang = static_method(
            "hang",
            &[],
            "V",
            0,
            0,
            [insn::goto(2), insn::nop(), insn::goto(-2)].concat(),
        );
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", None)
            .with_method(pemanggil)
            .with_method(hang)])
    };

    let probe = build(0);
    let index = probe.method_index("LContoh;", "hang");
    let bytes = build(index).bytes;

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "pemanggil")
        .expect_err("perulangan tanpa henti harus ditolak");
    match error {
        DexError::ExecutionLimit { location, .. } => assert_eq!(
            location.as_deref(),
            Some("LContoh;->hang"),
            "lokasi harus menunjuk method yang berputar"
        ),
        other => panic!("galat tak terduga: {other:?}"),
    }
}
#[test]
fn pembagian_dengan_nol_dilaporkan() {
    let bytes = build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(
        static_method(
            "bagi",
            &["I", "I"],
            "I",
            2,
            2,
            [insn::div_int(0, 0, 1), insn::return_(0)].concat(),
        ),
    )])
    .bytes;

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method_with_args("LContoh;", "bagi", &[10, 0])
        .expect_err("pembagian dengan nol harus ditolak");
    assert!(
        matches!(error, DexError::DivisionByZero),
        "error: {error:?}"
    );
}

#[test]
fn opcode_di_luar_cakupan_dilaporkan_eksplisit() {
    // 0xfa = invoke-polymorphic, belum didukung.
    let body = vec![0xfa, 0, 0x0e];
    let bytes =
        build_dex_with_symbols(
            &[ClassSpec::new("LContoh;", None).with_method(static_method(
                "aneh",
                &[],
                "V",
                1,
                0,
                body,
            ))],
        )
        .bytes;

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "aneh")
        .expect_err("opcode tak didukung harus ditolak");
    match error {
        DexError::InvalidInstruction { opcode, .. } => assert_eq!(opcode, 0xfa),
        other => panic!("error tak terduga: {other:?}"),
    }
}

#[test]
fn const_string_mengalokasikan_objek_string() {
    let build = |index: u16| {
        let method = static_method(
            "sapa",
            &[],
            "Ljava/lang/String;",
            1,
            0,
            [insn::const_string(0, index), insn::return_object(0)].concat(),
        )
        .literal("halo");
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(method)])
    };

    let probe = build(0);
    let index = probe.string_index("halo");
    let bytes = build(index).bytes;

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "sapa")
        .expect("eksekusi harus berhasil");

    let reference = outcome
        .return_value
        .expect("harus mengembalikan nilai")
        .as_ref()
        .expect("harus berupa referensi objek");
    assert_eq!(engine.heap.string(reference), Some("halo"));
}

#[test]
fn new_instance_membuat_objek_dengan_tipe_yang_benar() {
    let build = |type_index: u16| {
        let method = static_method(
            "buat",
            &[],
            "LContoh;",
            1,
            0,
            [insn::new_instance(0, type_index), insn::return_object(0)].concat(),
        );
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(method)])
    };

    let probe = build(0);
    let type_index = probe.type_index("LContoh;");
    let bytes = build(type_index).bytes;

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "buat")
        .expect("eksekusi harus berhasil");

    let reference = outcome
        .return_value
        .expect("harus mengembalikan nilai")
        .as_ref()
        .expect("harus berupa referensi objek");
    assert_eq!(
        engine.heap.get(reference),
        Some(&Object::Instance {
            type_name: "LContoh;".to_string(),
            fields: std::collections::HashMap::new(),
        })
    );
}

#[derive(Debug)]
struct ResolverAbs;

impl NativeResolver for ResolverAbs {
    fn invoke_static(
        &self,
        class_descriptor: &str,
        method_name: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        if class_descriptor != "Ljava/lang/Math;" || method_name != "abs" {
            return None;
        }
        Some(
            args.first()
                .and_then(|value| value.as_int())
                .map(|value| Value::Int(value.wrapping_abs()))
                .ok_or_else(|| "butuh 1 argumen int".to_string()),
        )
    }
}

fn dex_memanggil_math_abs() -> Vec<u8> {
    let build = |method_index: u16| {
        let math = ClassSpec::new("Ljava/lang/Math;", None).with_method(MethodSpec::new(
            "abs",
            ACC_PUBLIC | ACC_STATIC,
            &["I"],
            "I",
        ));
        let caller = static_method(
            "panggil",
            &[],
            "I",
            1,
            0,
            [
                insn::const_16(0, -12),
                insn::invoke_static(method_index, &[0]),
                insn::move_result(0),
                insn::return_(0),
            ]
            .concat(),
        );
        build_dex_with_symbols(&[math, ClassSpec::new("LContoh;", None).with_method(caller)])
    };

    let probe = build(0);
    let index = probe.method_index("Ljava/lang/Math;", "abs");
    build(index).bytes
}

#[test]
fn memanggil_method_native_lewat_resolver() {
    let bytes = dex_memanggil_math_abs();
    let mut engine = engine(&bytes);
    engine.set_native_resolver(Box::new(ResolverAbs));

    let outcome = engine
        .execute_method("LContoh;", "panggil")
        .expect("eksekusi harus berhasil");

    assert_eq!(outcome.return_value, Some(Value::Int(12)));
}

#[test]
fn method_native_tanpa_resolver_dilaporkan_tidak_tersedia() {
    let bytes = dex_memanggil_math_abs();
    let mut engine = engine(&bytes);

    let error = engine
        .execute_method("LContoh;", "panggil")
        .expect_err("tanpa resolver, method native harus gagal");
    assert!(matches!(error, DexError::NoCode { .. }), "error: {error:?}");
}

#[test]
fn argumen_native_bukan_int_dilaporkan() {
    let build = |method_index: u16| {
        let math = ClassSpec::new("Ljava/lang/Math;", None).with_method(MethodSpec::new(
            "abs",
            ACC_PUBLIC | ACC_STATIC,
            &["I"],
            "I",
        ));
        let caller = static_method(
            "panggil",
            &[],
            "I",
            1,
            0,
            [
                insn::const_string(0, 0),
                insn::invoke_static(method_index, &[0]),
                insn::move_result(0),
                insn::return_(0),
            ]
            .concat(),
        )
        .literal("bukan int");
        build_dex_with_symbols(&[math, ClassSpec::new("LContoh;", None).with_method(caller)])
    };

    let probe = build(0);
    let index = probe.method_index("Ljava/lang/Math;", "abs");
    let bytes = build(index).bytes;

    let mut engine = engine(&bytes);
    engine.set_native_resolver(Box::new(ResolverAbs));

    let error = engine
        .execute_method("LContoh;", "panggil")
        .expect_err("argumen salah tipe harus gagal");
    assert!(
        matches!(error, DexError::NativeFailure { .. }),
        "error: {error:?}"
    );
}

#[test]
fn move_object_semua_format_menyalin_referensi() {
    let bytes = build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(
        MethodSpec::new(
            "jalankan",
            ACC_PUBLIC | ACC_STATIC,
            &[],
            "Ljava/lang/Object;",
        )
        .registers(4)
        .body(
            [
                insn::new_instance(0, 0),
                insn::move_object(1, 0),
                insn::move_object_from16(2, 1),
                insn::move_object_16(3, 2),
                insn::return_object(3),
            ]
            .concat(),
        ),
    )])
    .bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("move-object harus berhasil");
    assert!(matches!(outcome.return_value, Some(Value::Ref(_))));
}
/// Resolver yang menandai loop utama idle, seperti `Looper.loop()` pada
/// runtime kooperatif.
#[derive(Debug)]
struct ResolverIdle;

impl NativeResolver for ResolverIdle {
    fn invoke_static(
        &self,
        _class_descriptor: &str,
        _method_name: &str,
        _args: &[Value],
    ) -> Option<Result<Value, String>> {
        Some(Err(arunika_dex_engine::MAIN_LOOP_IDLE.to_string()))
    }
}

#[test]
fn loop_utama_idle_menghentikan_eksekusi() {
    // `putarLoop` tidak punya badan bytecode, jadi panggilannya masuk ke
    // resolver seperti method framework.
    let build = |idle_index: u16| {
        let putar = static_method(
            "putar",
            &[],
            "V",
            0,
            0,
            [insn::invoke_static(idle_index, &[]), insn::return_void()].concat(),
        );
        let idle = MethodSpec::new("putarLoop", ACC_PUBLIC | ACC_STATIC, &[], "V");
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", None)
            .with_method(putar)
            .with_method(idle)])
    };
    let probe = build(0);
    let idle_index = probe.method_index("LContoh;", "putarLoop");
    let bytes = build(idle_index).bytes;
    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(ResolverIdle));

    let error = engine
        .execute_method("LContoh;", "putar")
        .expect_err("loop idle harus menghentikan eksekusi");
    assert!(
        matches!(error, DexError::MainLoopIdle),
        "galat tak terduga: {error:?}"
    );
}
