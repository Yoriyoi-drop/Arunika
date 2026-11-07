//! Uji tipe numerik 64-bit dan float (fase 1.5).
//!
//! Cakupan: `const-wide*`, `add-long`, `div-long`, `cmp-long`, `add-float`,
//! `add-double`, dan konversi antar tipe.

use arunika_dex_engine::{DexEngine, DexError, Value, parse_bytes};
use arunika_test_fixtures::dex::{ACC_PUBLIC, ACC_STATIC, ClassSpec, MethodSpec, build_dex, insn};

fn engine(insns: Vec<u16>, registers: u16, ret: &str) -> DexEngine {
    let bytes = build_dex(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new("jalankan", ACC_PUBLIC | ACC_STATIC, &[], ret)
                .registers(registers)
                .ins(0)
                .body(insns),
        ),
    ]);
    let dex = parse_bytes(&bytes, "numerik.dex").expect("DEX fixture harus terparse");
    DexEngine::new(dex)
}

fn jalankan(insns: Vec<u16>, registers: u16, ret: &str) -> Value {
    let mut engine = engine(insns, registers, ret);
    engine
        .execute_method("LContoh;", "jalankan")
        .expect("eksekusi harus berhasil")
        .return_value
        .expect("harus mengembalikan nilai")
}

#[test]
fn menjumlahkan_long() {
    let value = jalankan(
        [
            insn::const_wide_16(0, 40),
            insn::const_wide_16(2, 2),
            insn::add_long(0, 0, 2),
            insn::return_wide(0),
        ]
        .concat(),
        4,
        "J",
    );

    assert_eq!(value, Value::from_long(42));
}

#[test]
fn pembagian_long_dengan_nol_ditolak() {
    let mut engine = engine(
        [
            insn::const_wide_16(0, 10),
            insn::const_wide_16(2, 0),
            insn::div_long(0, 0, 2),
            insn::return_wide(0),
        ]
        .concat(),
        4,
        "J",
    );

    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("pembagian dengan nol harus gagal");
    assert!(matches!(error, DexError::DivisionByZero));
}

#[test]
fn cmp_long_mengembalikan_tanda() {
    let value = jalankan(
        [
            insn::const_wide_16(0, 5),
            insn::const_wide_16(2, 9),
            insn::cmp_long(0, 0, 2),
            insn::return_(0),
        ]
        .concat(),
        4,
        "I",
    );

    assert_eq!(value, Value::Int(-1));
}

#[test]
fn menjumlahkan_float() {
    // 1.0f = 0x3F800000, 2.0f = 0x40000000.
    let value = jalankan(
        [
            insn::const_high16(0, 0x3F80),
            insn::const_high16(1, 0x4000),
            insn::add_float(0, 0, 1),
            insn::return_(0),
        ]
        .concat(),
        2,
        "F",
    );

    assert_eq!(value.as_float(), Some(3.0));
}

#[test]
fn menjumlahkan_double() {
    // 1.5 = 0x3FF8000000000000, 2.25 = 0x4002000000000000.
    let value = jalankan(
        [
            insn::const_wide(0, 0x3FF8_0000_0000_0000u64 as i64),
            insn::const_wide(2, 0x4002_0000_0000_0000u64 as i64),
            insn::add_double(0, 0, 2),
            insn::return_wide(0),
        ]
        .concat(),
        4,
        "D",
    );

    assert_eq!(value.as_double(), Some(3.75));
}

#[test]
fn konversi_int_ke_long_dan_kembali() {
    let value = jalankan(
        [
            insn::const_16(0, 7),
            insn::int_to_long(0, 0),
            insn::long_to_int(1, 0),
            insn::return_(1),
        ]
        .concat(),
        2,
        "I",
    );

    assert_eq!(value, Value::Int(7));
}

#[test]
fn konversi_float_ke_int_memotong_pecahan() {
    // 2.9f = 0x4039999A.
    let value = jalankan(
        [
            insn::const_high16(0, 0x4039),
            insn::const_16(1, 0x199A),
            insn::add_int(0, 0, 1),
            insn::float_to_int(2, 0),
            insn::return_(2),
        ]
        .concat(),
        3,
        "I",
    );

    assert_eq!(value, Value::Int(2));
}

#[test]
fn int_ke_float_mempertahankan_nilai() {
    let value = jalankan(
        [
            insn::const_16(0, 12),
            insn::int_to_float(0, 0),
            insn::return_(0),
        ]
        .concat(),
        1,
        "F",
    );

    assert_eq!(value.as_float(), Some(12.0));
}
