use super::legacy_barrier_scope;
use windows::Win32::Graphics::Direct3D12::*;

#[test]
fn legacy_barrier_scope_maps_direct_compute_and_copy_queues() {
    let direct = D3D12_COMMAND_LIST_TYPE_DIRECT;
    let compute = D3D12_COMMAND_LIST_TYPE_COMPUTE;
    let copy = D3D12_COMMAND_LIST_TYPE_COPY;

    assert_eq!(
        legacy_barrier_scope(D3D12_RESOURCE_STATE_UNORDERED_ACCESS, direct),
        Some((
            D3D12_BARRIER_SYNC_ALL_SHADING,
            D3D12_BARRIER_ACCESS_UNORDERED_ACCESS,
            D3D12_BARRIER_LAYOUT_UNORDERED_ACCESS,
        ))
    );
    assert_eq!(
        legacy_barrier_scope(D3D12_RESOURCE_STATE_UNORDERED_ACCESS, compute),
        Some((
            D3D12_BARRIER_SYNC_COMPUTE_SHADING,
            D3D12_BARRIER_ACCESS_UNORDERED_ACCESS,
            D3D12_BARRIER_LAYOUT_UNORDERED_ACCESS,
        ))
    );
    assert_eq!(
        legacy_barrier_scope(D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE, direct),
        Some((
            D3D12_BARRIER_SYNC_NON_PIXEL_SHADING,
            D3D12_BARRIER_ACCESS_SHADER_RESOURCE,
            D3D12_BARRIER_LAYOUT_SHADER_RESOURCE,
        ))
    );
    assert_eq!(
        legacy_barrier_scope(D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, direct),
        Some((
            D3D12_BARRIER_SYNC_PIXEL_SHADING,
            D3D12_BARRIER_ACCESS_SHADER_RESOURCE,
            D3D12_BARRIER_LAYOUT_SHADER_RESOURCE,
        ))
    );
    assert_eq!(
        legacy_barrier_scope(D3D12_RESOURCE_STATE_COMMON, copy),
        Some((
            D3D12_BARRIER_SYNC_COPY,
            D3D12_BARRIER_ACCESS_COMMON,
            D3D12_BARRIER_LAYOUT_COMMON,
        ))
    );
    assert_eq!(
        legacy_barrier_scope(D3D12_RESOURCE_STATE_COPY_SOURCE, copy),
        Some((
            D3D12_BARRIER_SYNC_COPY,
            D3D12_BARRIER_ACCESS_COPY_SOURCE,
            D3D12_BARRIER_LAYOUT_COPY_SOURCE,
        ))
    );
    assert_eq!(
        legacy_barrier_scope(
            D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE | D3D12_RESOURCE_STATE_COPY_SOURCE,
            compute,
        ),
        Some((
            D3D12_BARRIER_SYNC_COMPUTE_SHADING | D3D12_BARRIER_SYNC_COPY,
            D3D12_BARRIER_ACCESS_SHADER_RESOURCE | D3D12_BARRIER_ACCESS_COPY_SOURCE,
            D3D12_BARRIER_LAYOUT_GENERIC_READ,
        ))
    );
    assert_eq!(legacy_barrier_scope(D3D12_RESOURCE_STATE_DEPTH_WRITE, direct), None);
}
