//! Heap typed buffer load, `MTBUF` form.
//!
//! The captured form is `s_lshl_b32` by 5, `s_load_dwordx8` from the class-5
//! heap base, then `tbuffer_load_format_x` with OFFEN and no IDXEN, format 24
//! (16_16_SNORM). This is the fixture that reaches the per-format conversion
//! table in `emit_mtbuf`.

use super::Emitter;
use crate::rdna2::disassemble;
use crate::{ShaderExtract, Stage};

fn fssm_rows(rows: &[[u32; 5]]) -> Vec<u8> {
    let mut program = vec![0; 0x100];
    program[..8].copy_from_slice(b"FSSM\x01\x01\0\0");
    let offset = (program.len() - 16) as u64;
    program[0xe8..0xe8 + 8].copy_from_slice(&offset.to_le_bytes());
    program[0xf8..0xf8 + 4].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    program.extend(rows.iter().flatten().copied().flat_map(u32::to_le_bytes));
    let size = (program.len() - 16) as u32;
    program[8..12].copy_from_slice(&size.to_le_bytes());
    program
}

fn heap_extract(isa: &[u8]) -> ShaderExtract {
    ShaderExtract { flat_inputs: Vec::new(), flat_outputs: Vec::new(),
        stage: Stage::Compute,
        shader_model: (6, 6),
        required_features: Some(1 << 25),
        entry: Some("main".into()),
        isa: isa.to_vec(),
        isa_data: isa.to_vec(),
        isa_offset_in_fssm: 0,
        isa_preamble: [0, 64, 1, 1],
        // Class 5, one dword, hardware register s10.
        fssm: fssm_rows(&[[5, 0, 1, 0, 10]]),
        bitcode_stripped: true,
        fspx_config: None,
        ps09_pixel_inputs: None,
        psv_threads: None,
        input_sig: Vec::new(),
        output_sig: Vec::new(),
        primitive_sig: Vec::new(),
        has_input_sig: false,
        has_output_sig: false,
        has_root_sig: true,
        has_psv0: false,
        resources: Vec::new(),
        gs_info: None,
        mesh_info: None,
        root_sig: Some(tuxbox_dxbc::rootsig::RootSig {
            version: 1,
            flags: 0x400,
            params: Vec::new(),
            num_static_samplers: 0,
            static_samplers: Vec::new(),
        }),
    }
}

/// s4 = s4 << 5; s[8:15] = heap[s4]; then the typed load itself.
fn mtbuf_words(first: u32, second: u32) -> [u32; 5] {
    [
        0x8f04_8504, // s_lshl_b32 s4, s4, 5
        0xf408_0205, // s_load_dwordx4 s[8:11], s[10:11], s4
        0x0800_0000,
        first,
        second,
    ]
}

fn lower(words: &[u32]) -> Result<String, String> {
    let ex = heap_extract(&words.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<_>>());
    let insns = disassemble(&ex.isa);
    let mut em = Emitter::new(&ex, insns.clone());
    em.resolve_resources()?;
    for (idx, ins) in insns.iter().enumerate() {
        em.emit_insn(idx, ins, false, None)?;
    }
    let mut hlsl = em.resource_decls()?;
    hlsl.push_str(&em.heap_handle_decls());
    hlsl.push_str(&em.body);
    Ok(hlsl)
}

/// MTBUF first word: offset 0, OFFEN, op 0 (load_format_x), format in bits 19-25.
fn mtbuf_first(format: u32) -> u32 {
    (0x3a << 26) | (format << 19) | 0x1000
}

/// MTBUF second word: vdata 4, vaddr 5, srsrc s8, soffset the constant zero
/// (selector 128), because `emit_mtbuf` requires a proven-zero offset.
const MTBUF_SECOND: u32 = 0x8002_0504;

#[test]
fn typed_load_of_16_16_snorm_converts_one_channel() {
    let hlsl = match lower(&mtbuf_words(mtbuf_first(24), MTBUF_SECOND)) {
        Ok(hlsl) => hlsl,
        Err(error) => panic!("refused: {error}"),
    };
    assert!(hlsl.contains("/ 32767.0f"), "{hlsl}");
    assert!(hlsl.contains(">> 16"), "{hlsl}");
}

#[test]
fn a_typed_store_is_still_refused() {
    // Op 4 is `tbuffer_store_format_x`. The bucket that refuses stores is worth
    // 577 of the 4,496 refusals, so this test marks where that work starts.
    let store = mtbuf_first(24) | (4 << 16);
    let error = match lower(&mtbuf_words(store, MTBUF_SECOND)) {
        Ok(hlsl) => panic!("store translated: {hlsl}"),
        Err(error) => error,
    };
    assert!(
        error.contains("unsupported typed buffer load addressing or modifiers"),
        "{error}"
    );
}
