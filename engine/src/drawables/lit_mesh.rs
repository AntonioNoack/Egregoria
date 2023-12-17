use crate::pbuffer::PBuffer;
use crate::{
    CompiledModule, Drawable, GfxContext, IndexType, Material, MaterialID, MeshInstance,
    MeshVertex, MikktGeometry, PipelineBuilder, RenderParams, Texture, Uniform, TL,
};
use geom::Matrix4;
use smallvec::SmallVec;
use std::sync::Arc;
use wgpu::{
    BindGroupLayout, BufferUsages, Device, IndexFormat, RenderPass, RenderPipeline,
    VertexBufferLayout,
};

pub struct MeshBuilder<const PERSISTENT: bool> {
    pub(crate) vertices: Vec<MeshVertex>,
    pub(crate) indices: Vec<IndexType>,
    pub(crate) vi_buffers: Option<Box<(PBuffer, PBuffer)>>,
    /// List of materialID and the starting offset
    pub(crate) materials: SmallVec<[(MaterialID, u32); 1]>,
}

impl<const PERSISTENT: bool> MikktGeometry for MeshBuilder<PERSISTENT> {
    fn num_faces(&self) -> usize {
        self.indices.len() / 3
    }

    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }

    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        let i = self.indices[face * 3 + vert] as usize;
        self.vertices[i].position
    }

    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        let i = self.indices[face * 3 + vert] as usize;
        self.vertices[i].normal.into()
    }

    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        let i = self.indices[face * 3 + vert] as usize;
        self.vertices[i].uv
    }

    fn set_tangent_encoded(&mut self, tangent: [f32; 4], face: usize, vert: usize) {
        let i = self.indices[face * 3 + vert] as usize;
        self.vertices[i].tangent = tangent;
    }
}

impl<const PERSISTENT: bool> MeshBuilder<PERSISTENT> {
    pub fn new(mat: MaterialID) -> Self {
        Self {
            vertices: vec![],
            indices: vec![],
            vi_buffers: PERSISTENT.then(|| {
                Box::new((
                    PBuffer::new(BufferUsages::VERTEX),
                    PBuffer::new(BufferUsages::INDEX),
                ))
            }),
            materials: smallvec::smallvec![(mat, 0)],
        }
    }

    pub fn new_without_mat() -> Self {
        Self {
            vertices: vec![],
            indices: vec![],
            vi_buffers: PERSISTENT.then(|| {
                Box::new((
                    PBuffer::new(BufferUsages::VERTEX),
                    PBuffer::new(BufferUsages::INDEX),
                ))
            }),
            materials: Default::default(),
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }

    pub fn extend(&mut self, vertices: &[MeshVertex], indices: &[IndexType]) -> &mut Self {
        let offset = self.vertices.len() as IndexType;
        self.vertices.extend_from_slice(vertices);
        self.indices.extend(indices.iter().map(|x| x + offset));
        self
    }

    /// Sets the material for all future indice pushes
    pub fn set_material(&mut self, material: MaterialID) {
        let n = self.indices.len() as u32;
        self.materials.push((material, n));
    }

    #[inline(always)]
    pub fn extend_with(&mut self, f: impl FnOnce(&mut Vec<MeshVertex>, &mut dyn FnMut(IndexType))) {
        let offset = self.vertices.len() as IndexType;
        let vertices = &mut self.vertices;
        let indices = &mut self.indices;
        let mut x = move |index: IndexType| {
            indices.push(index + offset);
        };
        f(vertices, &mut x);
    }

    pub fn compute_tangents(&mut self) {
        if !crate::geometry::generate_tangents(self) {
            log::warn!("failed to generate tangents");
        }
    }

    pub fn build(&mut self, gfx: &GfxContext) -> Option<Mesh> {
        if self.vertices.is_empty() {
            return None;
        }

        let mut tmpv;
        let mut tmpi;
        let vbuffer;
        let ibuffer;

        if PERSISTENT {
            let x = self.vi_buffers.as_deref_mut().unwrap();
            vbuffer = &mut x.0;
            ibuffer = &mut x.1;
        } else {
            tmpv = PBuffer::new(BufferUsages::VERTEX);
            tmpi = PBuffer::new(BufferUsages::INDEX);
            vbuffer = &mut tmpv;
            ibuffer = &mut tmpi;
        }

        vbuffer.write(gfx, bytemuck::cast_slice(&self.vertices));
        ibuffer.write(gfx, bytemuck::cast_slice(&self.indices));

        // convert materials to mesh format (from offsets to lengths)
        let mut materials = SmallVec::with_capacity(self.materials.len());
        let mut mats = self.materials.iter().peekable();
        while let Some((mat, start)) = mats.next() {
            let end = mats
                .peek()
                .map(|(_, x)| *x)
                .unwrap_or(self.indices.len() as u32);
            let l = end - start;
            if l == 0 {
                continue;
            }
            materials.push((*mat, l));
        }

        Some(Mesh {
            vertex_buffer: vbuffer.inner()?,
            index_buffer: ibuffer.inner()?,
            materials,
            skip_depth: false,
        })
    }
}

#[derive(Clone)]
pub struct Mesh {
    pub vertex_buffer: Arc<wgpu::Buffer>,
    pub index_buffer: Arc<wgpu::Buffer>,
    /// List of materialID and the indice length
    pub materials: SmallVec<[(MaterialID, u32); 1]>,
    pub skip_depth: bool,
}

impl Mesh {
    /// Returns an iterator over the materials used by this mesh
    /// The iterator returns the materialID, the index offset and the number of indices for that material
    pub fn iter_materials(&self) -> impl Iterator<Item = (MaterialID, u32, u32)> + '_ {
        let mut offset = 0;
        self.materials.iter().map(move |(mat, n)| {
            let ret = (*mat, offset, *n);
            offset += *n;
            ret
        })
    }
}

#[derive(Clone, Copy, Hash)]
pub(crate) struct MeshPipeline {
    pub(crate) instanced: bool,
    pub(crate) alpha: bool,
    pub(crate) smap: bool,
    pub(crate) depth: bool,
}

const VB_INSTANCED: &[VertexBufferLayout] = &[MeshVertex::desc(), MeshInstance::desc()];
const VB: &[VertexBufferLayout] = &[MeshVertex::desc()];

impl PipelineBuilder for MeshPipeline {
    fn build(
        &self,
        gfx: &GfxContext,
        mut mk_module: impl FnMut(&str) -> CompiledModule,
    ) -> RenderPipeline {
        let vert = if self.instanced {
            mk_module("instanced_mesh.vert")
        } else {
            mk_module("lit_mesh.vert")
        };

        let vb: &[VertexBufferLayout] = if self.instanced { VB_INSTANCED } else { VB };

        if !self.depth {
            let frag = mk_module("pixel.frag");
            return gfx.color_pipeline(
                "lit_mesh",
                &[
                    &gfx.projection.layout,
                    &Uniform::<RenderParams>::bindgroup_layout(&gfx.device),
                    &Material::bindgroup_layout(&gfx.device),
                    &bg_layout_litmesh(&gfx.device),
                ],
                vb,
                &vert,
                &frag,
            );
        }

        if !self.alpha {
            return gfx.depth_pipeline(vb, &vert, None, self.smap);
        }

        let frag = mk_module("alpha_discard.frag");
        gfx.depth_pipeline_bglayout(
            vb,
            &vert,
            Some(&frag),
            self.smap,
            &[
                &gfx.projection.layout,
                &Material::bindgroup_layout(&gfx.device),
            ],
        )
    }
}

impl Drawable for Mesh {
    fn draw<'a>(&'a self, gfx: &'a GfxContext, rp: &mut RenderPass<'a>) {
        rp.set_bind_group(1, &gfx.render_params.bindgroup, &[]);
        rp.set_bind_group(3, &gfx.simplelit_bg, &[]);
        rp.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        rp.set_index_buffer(self.index_buffer.slice(..), IndexFormat::Uint32);

        for (mat, offset, length) in self.iter_materials() {
            let mat = gfx.material(mat);
            rp.set_pipeline(gfx.get_pipeline(MeshPipeline {
                instanced: false,
                alpha: false,
                smap: false,
                depth: false,
            }));
            rp.set_bind_group(2, &mat.bg, &[]);
            rp.draw_indexed(offset..offset + length, 0, 0..1);

            gfx.perf.drawcall(length / 3);
        }
    }

    fn draw_depth<'a>(
        &'a self,
        gfx: &'a GfxContext,
        rp: &mut RenderPass<'a>,
        shadow_cascade: Option<&Matrix4>,
    ) {
        if self.skip_depth {
            return;
        }
        rp.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        rp.set_index_buffer(self.index_buffer.slice(..), IndexFormat::Uint32);

        for (mat, offset, length) in self.iter_materials() {
            let mat = gfx.material(mat);
            rp.set_pipeline(gfx.get_pipeline(MeshPipeline {
                instanced: false,
                alpha: mat.transparent,
                smap: shadow_cascade.is_some(),
                depth: true,
            }));

            if mat.transparent {
                rp.set_bind_group(1, &mat.bg, &[]);
            }
            rp.draw_indexed(offset..offset + length, 0, 0..1);

            gfx.perf
                .depth_drawcall(length / 3, shadow_cascade.is_some());
        }
    }
}

pub struct LitMeshDepth;
pub struct LitMeshDepthSMap;

pub fn bg_layout_litmesh(device: &Device) -> BindGroupLayout {
    Texture::bindgroup_layout(
        device,
        [
            TL::Float,
            TL::Float,
            TL::DepthArray,
            TL::Cube,
            TL::Cube,
            TL::Float,
            TL::UInt,
            TL::UInt,
        ],
    )
}
