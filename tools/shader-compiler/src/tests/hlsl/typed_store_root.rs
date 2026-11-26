//! A typed buffer store through a root descriptor that the root calls read-only.
//!
//! The bucket is 577 of the 4,496 ProjectScarlett refusals. This fixture reaches
//! the refusal so the message can name the table it refused, which is what a
//! session log needs to say how the bucket splits.

use super::Emitter;
use crate::rdna2::disassemble;
use crate::{ShaderExtract, Stage};

/// Load the descriptor with `s_load_dwordx4 s[8:11], s[0:1], 0`. The decoder reads
/// SOFFSET from the top seven bits of the second word, where 0x7d means no offset.
const LOAD_DESCRIPTOR: [u32; 2] = [0xf408_0200, 0xfa00_0000];
/// `buffer_store_format_x` (MUBUF op 4), vdata v4, vaddr v0, srsrc s8, soffset 0.
const TYPED_STORE: [u32; 2] = [(0x38 << 26) | (4 << 18), 0x8002_0004];
const ENDPGM: u32 = 0xbf81_0000;

fn fixture() -> ShaderExtract {
    let words: Vec<u32> = LOAD_DESCRIPTOR
        .iter()
        .chain(&TYPED_STORE)
        .chain(&[ENDPGM])
        .copied()
        .collect();
    let isa: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut ex = crate::hlsl::mesh::heap_extract(&[[3, 8, 1, 0, 4]], 0, &isa);
    ex.stage = Stage::Compute;
    ex.psv_threads = Some([64, 1, 1]);
    ex.required_features = None;
    // A typed SRV at t0, which is what PSV row 6 means.
    ex.resources.push(crate::psv::PsvResource { res_type: 6, space: 0, lower: 0, upper: 0 });
    let root = ex.root_sig.as_mut().unwrap();
    root.version = 2;
    root.params.push(crate::meta::RootParam {
        visibility: 0,
        kind: crate::meta::RootParamKind::Table(vec![crate::meta::RootRange {
            range_type: 0,
            num: 1,
            base_register: 0,
            space: 0,
            offset_in_table: 0,
        }]),
    });
    ex
}

#[test]
#[ignore = "needs a root table the analysis accepts as a typed buffer: it refuses with 'mubuf at 0x8: buffer in table of kind None'. See LB-18 in LOGBLOCK.md"]
fn a_typed_store_through_a_read_only_root_names_its_table() {
    let ex = fixture();
    let insns = disassemble(&ex.isa);
    let mut em = Emitter::new(&ex, insns.clone());
    let error = match em.resolve_resources() {
        Ok(()) => {
            let mut failure = None;
            for (idx, ins) in insns.iter().enumerate() {
                if let Err(error) = em.emit_insn(idx, ins, false, None) {
                    failure = Some(error);
                    break;
                }
            }
            match failure {
                Some(error) => error,
                None => panic!("the typed store translated:\n{}", em.body),
            }
        }
        Err(error) => error,
    };
    assert!(error.contains("non-UAV table"), "{error}");
    assert!(error.contains("buffer_store_format_x"), "{error}");
}
