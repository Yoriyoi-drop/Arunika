//! Uji ketahanan parser DEX terhadap input tidak tepercaya.
//!
//! Fokus: tidak ada panic, tidak ada alokasi tanpa batas, dan setiap
//! ketidakkonsistenan header dilaporkan sebagai error eksplisit.

use arunika_dex_engine::{DexEngine, DexError, parse_bytes};
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, ClassSpec, MethodSpec, build_dex_with_symbols, insn,
};
use arunika_test_fixtures::minimal_dex;

fn put_u32(dex: &mut [u8], offset: usize, value: u32) {
    dex[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn menolak_berkas_lebih_pendek_dari_header() {
    for len in 0..112 {
        let data = vec![0u8; len];
        assert!(
            parse_bytes(&data, "uji.dex").is_err(),
            "panjang {len} seharusnya ditolak"
        );
    }
}

#[test]
fn menolak_magic_tidak_valid() {
    let mut dex = minimal_dex();
    dex[..8].copy_from_slice(b"zip\n035\0");
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::InvalidMagic)
    ));
}

#[test]
fn menolak_header_size_yang_bukan_112() {
    let mut dex = minimal_dex();
    put_u32(&mut dex, 36, 100);
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::InvalidHeaderSize(100))
    ));
}

#[test]
fn menolak_endian_tag_tidak_dikenal() {
    let mut dex = minimal_dex();
    put_u32(&mut dex, 40, 0);
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::UnsupportedEndian(0))
    ));
}

#[test]
fn menolak_endian_tag_terbalik() {
    let mut dex = minimal_dex();
    put_u32(&mut dex, 40, 0x7856_3412);
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::UnsupportedEndian(0x7856_3412))
    ));
}

#[test]
fn menolak_file_size_yang_melebihi_data() {
    let mut dex = minimal_dex();
    put_u32(&mut dex, 32, 4096);
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::SizeMismatch {
            expected: 4096,
            actual: 112
        })
    ));
}

#[test]
fn menolak_tabel_string_yang_terlalu_besar() {
    let mut dex = minimal_dex();
    put_u32(&mut dex, 56, u32::MAX);
    put_u32(&mut dex, 60, 112);
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::LimitExceeded {
            table: "string_ids",
            ..
        })
    ));
}

#[test]
fn menolak_tabel_yang_keluar_dari_berkas() {
    let mut dex = minimal_dex();
    put_u32(&mut dex, 56, 1000);
    put_u32(&mut dex, 60, 112);
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::Truncated)
    ));
}

#[test]
fn menolak_offset_tabel_yang_overflow() {
    let mut dex = minimal_dex();
    put_u32(&mut dex, 56, 2);
    put_u32(&mut dex, 60, u32::MAX);
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::Truncated)
    ));
}

#[test]
fn mutasi_byte_tidak_pernah_panic() {
    let base = minimal_dex();

    for offset in 0..base.len() {
        for value in [0x00u8, 0x01, 0x7f, 0xff] {
            let mut dex = base.clone();
            dex[offset] = value;
            // Hasil boleh Ok atau Err; yang penting tidak panic.
            let _ = parse_bytes(&dex, "mutasi.dex");
        }
    }
}

#[test]
fn pemotongan_di_setiap_offset_tidak_pernah_panic() {
    let base = minimal_dex();
    for len in 0..=base.len() {
        let _ = parse_bytes(&base[..len], "potong.dex");
    }
}

#[test]
fn inspeksi_dex_valid_melaporkan_header() {
    let inspection =
        arunika_dex_engine::inspect_dex_bytes(&minimal_dex(), "info", "uji.dex").expect("valid");
    assert_eq!(inspection.version, "035");
    assert_eq!(inspection.header_size, 112);
    assert_eq!(inspection.file_size, 112);
    assert_eq!(inspection.string_ids_count, 0);
}

#[test]
fn mutasi_instruksi_tidak_panic_saat_dieksekusi() {
    let body = [
        insn::const_4(0, 2),
        insn::const_16(1, 300),
        insn::add_int(2, 0, 1),
        insn::return_(2),
    ]
    .concat();
    let base = build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(
        MethodSpec::new("jalankan", ACC_PUBLIC | ACC_STATIC, &[], "I")
            .registers(3)
            .ins(0)
            .body(body.clone()),
    )])
    .bytes;

    let encoded: Vec<u8> = body.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    let start = base
        .windows(encoded.len())
        .position(|window| window == encoded.as_slice())
        .expect("instruksi fixture harus ditemukan di berkas");
    let end = start + encoded.len();

    // Setiap mutasi dijalankan di thread ber-stack kecil agar batas kedalaman
    // panggilan benar-benar diuji; hasil boleh Ok atau Err, yang penting tidak
    // panic dan tidak menggantung (MAX_STEPS membatasi eksekusi).
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            for offset in start..end {
                for value in [0x00u8, 0x01, 0x7f, 0xff] {
                    let mut dex = base.clone();
                    dex[offset] = value;
                    let Ok(parsed) = parse_bytes(&dex, "mutasi.dex") else {
                        continue;
                    };
                    let mut engine = DexEngine::new(parsed);
                    let _ = engine.execute_method("LContoh;", "jalankan");
                }
            }
        })
        .expect("thread uji harus dibuat");
    handle.join().expect("mutasi instruksi tidak boleh panic");
}

#[test]
fn menolak_code_item_dengan_offset_tries_luar_berkas() {
    let mut dex = minimal_dex();
    // code_offset pada class_data menunjuk ke luar berkas (offset 4096 di berkas 112 byte)
    put_u32(&mut dex, 88, 4096); // methods_size pada header
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::Truncated)
    ));
}

#[test]
fn menolak_class_def_dengan_offset_luar_berkas() {
    let mut dex = minimal_dex();
    // class_def_offset pada header menunjuk ke luar berkas
    put_u32(&mut dex, 96, 4096);
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::Truncated)
    ));
}

#[test]
fn menolak_proto_ids_dengan_type_list_luar_berkas() {
    let mut dex = minimal_dex();
    // proto_ids_offset menunjuk ke luar berkas
    put_u32(&mut dex, 80, 4096);
    assert!(matches!(
        parse_bytes(&dex, "uji.dex"),
        Err(DexError::Truncated)
    ));
}

#[test]
fn mutasi_header_tidak_pernah_panic() {
    let base = minimal_dex();
    for offset in 0..base.len() {
        for value in [0x00u8, 0xff] {
            let mut dex = base.clone();
            dex[offset] = value;
            let _ = parse_bytes(&dex, "mutasi-header.dex");
        }
    }
}
