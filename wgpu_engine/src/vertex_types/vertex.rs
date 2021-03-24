use crate::VBDesc;
use wgpu::VertexAttribute;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct ColoredVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

u8slice_impl!(ColoredVertex);

const ATTRS: &[VertexAttribute] = &wgpu::vertex_attr_array![0 => Float3, 1 => Float4];

impl VBDesc for ColoredVertex {
    fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<ColoredVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::InputStepMode::Vertex,
            attributes: ATTRS,
        }
    }
}
