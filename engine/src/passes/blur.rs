use wgpu::{
    Device, FragmentState, PipelineLayoutDescriptor, PrimitiveState, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, SurfaceConfiguration,
    TextureUsages, TextureView, VertexState,
};

use crate::{CompiledModule, Encoders, GfxContext, PipelineBuilder, Texture, TextureBuilder, TL};

const DOWNSCALE_PASSES: u32 = 2;

/// The blur pass to be used by the UI uses the "Dual Kawase Blur" algorithm as explained
/// in the SIGGRAPH 2015 paper "Bandwidth-efficient Rendering" by Marius Bjørge
///
/// It blurs the game color to be used as background for the UI.
/// The algorithm is as follows:
/// 1. Downsample the image to half resolution using bi-linear filtering
/// 2. Downsample then upsample using the equations from the paper
/// 3. Sample from the UI directly (bi-linearly filtered)
pub fn gen_ui_blur(gfx: &GfxContext, encs: &mut Encoders, frame: &TextureView) {
    profiling::scope!("ui blur pass");

    initial_downscale(gfx, encs, frame);

    for mip_level in 0..DOWNSCALE_PASSES {
        do_pass(
            gfx,
            encs,
            UIBlurPipeline::Downscale,
            mip_level,
            mip_level + 1,
        );
    }

    for mip_level in (0..DOWNSCALE_PASSES).rev() {
        do_pass(gfx, encs, UIBlurPipeline::Upscale, mip_level + 1, mip_level);
    }
}

// Simple downscale we can use mipmap gen
fn initial_downscale(gfx: &GfxContext, encs: &mut Encoders, frame: &TextureView) {
    let pipe = gfx.mipmap_gen.get_pipeline(&gfx.device, gfx.fbos.format);

    gfx.mipmap_gen.mipmap_one(
        &mut encs.end,
        &gfx.device,
        &pipe,
        frame,
        &gfx.fbos.ui_blur.mip_view(0),
        "ui blur",
    );
}

fn do_pass(gfx: &GfxContext, encs: &mut Encoders, pipeline: UIBlurPipeline, src: u32, dst: u32) {
    let tex = &gfx.fbos.ui_blur;
    let src = tex.mip_view(src);
    let dst = tex.mip_view(dst);

    let pipe = gfx.get_pipeline(pipeline);

    let bg = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("blur bindgroup"),
        layout: &pipe.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&src),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&gfx.linear_sampler),
            },
        ],
    });

    let mut blur_pass = encs.end.begin_render_pass(&RenderPassDescriptor {
        label: Some("ui blur pass"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &dst,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });

    blur_pass.set_pipeline(gfx.get_pipeline(UIBlurPipeline::Downscale));
    blur_pass.set_bind_group(0, &bg, &[]);
    blur_pass.draw(0..3, 0..1);
}

pub fn gen_blur_texture(device: &Device, sc: &SurfaceConfiguration) -> Texture {
    TextureBuilder::empty(sc.width / 2, sc.height / 2, 1, sc.format)
        .with_usage(TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING)
        .with_no_anisotropy()
        .with_fixed_mipmaps(1 + DOWNSCALE_PASSES)
        .build_no_queue(device)
}

#[derive(Hash)]
pub enum UIBlurPipeline {
    Downscale,
    Upscale,
}

impl PipelineBuilder for UIBlurPipeline {
    fn build(
        &self,
        gfx: &GfxContext,
        mut mk_module: impl FnMut(&str) -> CompiledModule,
    ) -> RenderPipeline {
        let bg = &mk_module("ui_blur");

        let render_pipeline_layout = gfx
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("ui blur"),
                bind_group_layouts: &[&Texture::bindgroup_layout(&gfx.device, [TL::Float])],
                push_constant_ranges: &[],
            });

        let color_states = [Some(wgpu::ColorTargetState {
            format: gfx.sc_desc.format,
            blend: None,
            write_mask: wgpu::ColorWrites::COLOR,
        })];

        let render_pipeline_desc = RenderPipelineDescriptor {
            label: Some("ui blur pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: VertexState {
                module: bg,
                entry_point: "vert",
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: bg,
                entry_point: match self {
                    UIBlurPipeline::Downscale => "downscale",
                    UIBlurPipeline::Upscale => "upscale",
                },
                targets: &color_states,
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
        };
        gfx.device.create_render_pipeline(&render_pipeline_desc)
    }
}
