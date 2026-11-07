//! Uji akses field dan array (fase 1.4).
//!
//! Cakupan: `sget`/`sput`, `iget`/`iput`, `new-array`, `array-length`,
//! `aget`/`aput`, serta penolakan indeks/ukuran yang tidak valid.

use arunika_dex_engine::{DexEngine, DexError, Value, parse_bytes};
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, ClassSpec, FieldSpec, MethodSpec, build_dex_with_symbols, insn,
};

fn engine(bytes: &[u8]) -> DexEngine {
    let dex = parse_bytes(bytes, "field.dex").expect("DEX fixture harus terparse");
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
fn field_statis_bertahan_antar_pemanggilan() {
    let build = |field_index: u16| {
        let setter = static_method(
            "set",
            &[],
            "V",
            1,
            0,
            [
                insn::const_4(0, 5),
                insn::sput(0, field_index),
                insn::return_void(),
            ]
            .concat(),
        );
        let getter = static_method(
            "get",
            &[],
            "I",
            1,
            0,
            [insn::sget(0, field_index), insn::return_(0)].concat(),
        );
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
            .with_field(FieldSpec::new("nilai", "I", true))
            .with_method(setter)
            .with_method(getter)])
    };

    let probe = build(0);
    let field_index = probe.field_index("LContoh;", "nilai");

    let mut engine = engine(&build(field_index).bytes);
    engine
        .execute_method("LContoh;", "set")
        .expect("sput harus berhasil");
    let outcome = engine
        .execute_method("LContoh;", "get")
        .expect("sget harus berhasil");

    assert_eq!(outcome.return_value, Some(Value::Int(5)));
}

#[test]
fn field_statis_belum_diisi_bernilai_nol() {
    let build = |field_index: u16| {
        let getter = static_method(
            "get",
            &[],
            "I",
            1,
            0,
            [insn::sget(0, field_index), insn::return_(0)].concat(),
        );
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
            .with_field(FieldSpec::new("nilai", "I", true))
            .with_method(getter)])
    };

    let probe = build(0);
    let field_index = probe.field_index("LContoh;", "nilai");

    let mut engine = engine(&build(field_index).bytes);
    let outcome = engine
        .execute_method("LContoh;", "get")
        .expect("sget harus berhasil");

    assert_eq!(outcome.return_value, Some(Value::Int(0)));
}

#[test]
fn field_instance_disimpan_pada_objek() {
    let build = |type_index: u16, field_index: u16| {
        let method = static_method(
            "jalankan",
            &[],
            "I",
            3,
            0,
            [
                insn::new_instance(0, type_index),
                insn::const_16(1, 9),
                insn::iput(1, 0, field_index),
                insn::iget(2, 0, field_index),
                insn::return_(2),
            ]
            .concat(),
        );
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
            .with_field(FieldSpec::new("x", "I", false))
            .with_method(method)])
    };

    let probe = build(0, 0);
    let type_index = probe.type_index("LContoh;");
    let field_index = probe.field_index("LContoh;", "x");

    let mut engine = engine(&build(type_index, field_index).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("iput/iget harus berhasil");

    assert_eq!(outcome.return_value, Some(Value::Int(9)));
}

#[test]
fn array_dapat_diisi_dan_dibaca() {
    let build = |int_type: u16| {
        let method = static_method(
            "jalankan",
            &[],
            "I",
            4,
            0,
            [
                insn::const_4(0, 3),
                insn::new_array(0, 0, int_type),
                insn::const_4(1, 7),
                insn::const_4(2, 1),
                insn::aput(1, 0, 2),
                insn::aget(3, 0, 2),
                insn::return_(3),
            ]
            .concat(),
        );
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method)
        ])
    };

    let int_type = build(0).type_index("I");
    let mut engine = engine(&build(int_type).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("operasi array harus berhasil");

    assert_eq!(outcome.return_value, Some(Value::Int(7)));
}

#[test]
fn array_length_mengembalikan_panjang() {
    let build = |int_type: u16| {
        let method = static_method(
            "jalankan",
            &[],
            "I",
            2,
            0,
            [
                insn::const_4(0, 4),
                insn::new_array(0, 0, int_type),
                insn::array_length(1, 0),
                insn::return_(1),
            ]
            .concat(),
        );
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method)
        ])
    };

    let int_type = build(0).type_index("I");
    let mut engine = engine(&build(int_type).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("array-length harus berhasil");

    assert_eq!(outcome.return_value, Some(Value::Int(4)));
}

#[test]
fn indeks_di_luar_batas_ditolak() {
    let build = |int_type: u16| {
        let method = static_method(
            "jalankan",
            &[],
            "I",
            4,
            0,
            [
                insn::const_4(0, 3),
                insn::new_array(0, 0, int_type),
                insn::const_4(1, 1),
                insn::const_4(2, 5),
                insn::aput(1, 0, 2),
                insn::return_(0),
            ]
            .concat(),
        );
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method)
        ])
    };

    let int_type = build(0).type_index("I");
    let mut engine = engine(&build(int_type).bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("indeks di luar batas harus ditolak");

    match error {
        DexError::ArrayIndexOutOfBounds { index, length } => {
            assert_eq!(index, 5);
            assert_eq!(length, 3);
        }
        other => panic!("error tak terduga: {other:?}"),
    }
}

#[test]
fn ukuran_array_negatif_ditolak() {
    let build = |int_type: u16| {
        let method = static_method(
            "jalankan",
            &[],
            "I",
            1,
            0,
            [
                insn::const_4(0, -1),
                insn::new_array(0, 0, int_type),
                insn::return_(0),
            ]
            .concat(),
        );
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method)
        ])
    };

    let int_type = build(0).type_index("I");
    let mut engine = engine(&build(int_type).bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("ukuran negatif harus ditolak");

    match error {
        DexError::NegativeArraySize { size } => assert_eq!(size, -1),
        other => panic!("error tak terduga: {other:?}"),
    }
}

#[test]
fn iget_pada_nilai_int_dilaporkan_sebagai_null_reference() {
    let bytes = build_dex_with_symbols(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
        .with_field(FieldSpec::new("x", "I", false))
        .with_method(static_method(
            "jalankan",
            &[],
            "I",
            2,
            0,
            [insn::const_4(0, 1), insn::iget(1, 0, 0), insn::return_(1)].concat(),
        ))])
    .bytes;

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("iget pada int harus gagal");

    match error {
        DexError::NullReference { pc } => assert_eq!(pc, 1),
        other => panic!("error tak terduga: {other:?}"),
    }
}
