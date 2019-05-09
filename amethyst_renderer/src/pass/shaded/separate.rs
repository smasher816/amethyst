//! Simple shaded pass

use derivative::Derivative;
use gfx::pso::buffer::ElemStride;
use gfx_core::state::{Blend, ColorMask};
use log::{debug, trace};

use amethyst_assets::AssetStorage;
use amethyst_core::{
    ecs::prelude::{Join, Read, ReadExpect, ReadStorage},
    transform::Transform,
};
use amethyst_error::Error;

use crate::{
    cam::{ActiveCamera, Camera},
    hidden::{Hidden, HiddenPropagate},
    light::Light,
    mesh::{Mesh, MeshHandle},
    mtl::{Material, MaterialDefaults},
    pass::{
        shaded_util::{set_light_args, setup_light_buffers},
        skinning::{create_skinning_effect, setup_skinning_buffers},
        util::{default_transparency, draw_mesh, get_camera, setup_textures, setup_vertex_args},
    },
    pipe::{
        pass::{Pass, PassData},
        DepthMode, Effect, NewEffect,
    },
    resources::AmbientColor,
    skinning::JointTransforms,
    tex::Texture,
    types::{Encoder, Factory},
    vertex::{Attributes, Normal, Position, Separate, TexCoord, VertexFormat},
    visibility::Visibility,
    Rgba,
};

use super::*;

static ATTRIBUTES: [Attributes<'static>; 3] = [
    Separate::<Position>::ATTRIBUTES,
    Separate::<Normal>::ATTRIBUTES,
    Separate::<TexCoord>::ATTRIBUTES,
];

/// Draw mesh with simple lighting technique
///
/// See the [crate level documentation](index.html) for information about interleaved and separate
/// passes.
#[derive(Derivative, Clone, Debug, PartialEq)]
#[derivative(Default)]
pub struct DrawShadedSeparate {
    skinning: bool,
    #[derivative(Default(value = "default_transparency()"))]
    transparency: Option<(ColorMask, Blend, Option<DepthMode>)>,
}

impl DrawShadedSeparate {
    /// Create instance of `DrawShaded` pass
    pub fn new() -> Self {
        Default::default()
    }

    /// Enable vertex skinning
    pub fn with_vertex_skinning(mut self) -> Self {
        self.skinning = true;
        self
    }

    /// Transparency is enabled by default.
    /// If you pass false to this function transparency will be disabled.
    ///
    /// If you pass true and this was disabled previously default settings will be reinstated.
    /// If you pass true and this was already enabled this will do nothing.
    pub fn with_transparency(mut self, input: bool) -> Self {
        if input {
            if self.transparency.is_none() {
                self.transparency = default_transparency();
            }
        } else {
            self.transparency = None;
        }
        self
    }

    /// Set transparency settings to custom values.
    pub fn with_transparency_settings(
        mut self,
        mask: ColorMask,
        blend: Blend,
        depth: Option<DepthMode>,
    ) -> Self {
        self.transparency = Some((mask, blend, depth));
        self
    }
}

impl<'a> PassData<'a> for DrawShadedSeparate {
    type Data = (
        Read<'a, ActiveCamera>,
        ReadStorage<'a, Camera>,
        Read<'a, AmbientColor>,
        Read<'a, AssetStorage<Mesh>>,
        Read<'a, AssetStorage<Texture>>,
        ReadExpect<'a, MaterialDefaults>,
        Option<Read<'a, Visibility>>,
        ReadStorage<'a, Hidden>,
        ReadStorage<'a, HiddenPropagate>,
        ReadStorage<'a, MeshHandle>,
        ReadStorage<'a, Material>,
        ReadStorage<'a, Transform>,
        ReadStorage<'a, Light>,
        ReadStorage<'a, JointTransforms>,
        ReadStorage<'a, Rgba>,
    );
}

impl Pass for DrawShadedSeparate {
    fn compile(&mut self, effect: NewEffect<'_>) -> Result<Effect, Error> {
        debug!("Building shaded pass");
        let mut builder = if self.skinning {
            create_skinning_effect(effect, FRAG_SRC)
        } else {
            effect.simple(VERT_SRC, FRAG_SRC)
        };
        debug!("Effect compiled, adding vertex/uniform buffers");
        builder
            .with_raw_vertex_buffer(
                Separate::<Position>::ATTRIBUTES,
                Separate::<Position>::size() as ElemStride,
                0,
            )
            .with_raw_vertex_buffer(
                Separate::<Normal>::ATTRIBUTES,
                Separate::<Normal>::size() as ElemStride,
                0,
            )
            .with_raw_vertex_buffer(
                Separate::<TexCoord>::ATTRIBUTES,
                Separate::<TexCoord>::size() as ElemStride,
                0,
            );
        if self.skinning {
            setup_skinning_buffers(&mut builder);
        }
        setup_vertex_args(&mut builder);
        setup_light_buffers(&mut builder);
        setup_textures(&mut builder, &TEXTURES);
        match self.transparency {
            Some((mask, blend, depth)) => builder.with_blended_output("color", mask, blend, depth),
            None => builder.with_output("color", Some(DepthMode::LessEqualWrite)),
        };
        builder.build()
    }

    fn apply<'a, 'b: 'a>(
        &'a mut self,
        encoder: &mut Encoder,
        effect: &mut Effect,
        _factory: Factory,
        (
            active,
            camera,
            ambient,
            mesh_storage,
            tex_storage,
            material_defaults,
            visibility,
            hidden,
            hidden_prop,
            mesh,
            material,
            transform,
            light,
            joints,
            rgba,
        ): <Self as PassData<'a>>::Data,
    ) {
        trace!("Drawing shaded pass");
        let camera = get_camera(active, &camera, &transform);

        set_light_args(effect, encoder, &light, &transform, &ambient, camera);

        match visibility {
            None => {
                for (joint, mesh, material, transform, rgba, _, _) in (
                    joints.maybe(),
                    &mesh,
                    &material,
                    &transform,
                    rgba.maybe(),
                    !&hidden,
                    !&hidden_prop,
                )
                    .join()
                {
                    draw_mesh(
                        encoder,
                        effect,
                        self.skinning,
                        mesh_storage.get(mesh),
                        joint,
                        &tex_storage,
                        Some(material),
                        &material_defaults,
                        rgba,
                        camera,
                        Some(transform),
                        &ATTRIBUTES,
                        &TEXTURES,
                    );
                }
            }
            Some(ref visibility) => {
                for (joint, mesh, material, transform, rgba, _) in (
                    joints.maybe(),
                    &mesh,
                    &material,
                    &transform,
                    rgba.maybe(),
                    &visibility.visible_unordered,
                )
                    .join()
                {
                    draw_mesh(
                        encoder,
                        effect,
                        self.skinning,
                        mesh_storage.get(mesh),
                        joint,
                        &tex_storage,
                        Some(material),
                        &material_defaults,
                        rgba,
                        camera,
                        Some(transform),
                        &ATTRIBUTES,
                        &TEXTURES,
                    );
                }

                for entity in &visibility.visible_ordered {
                    if let Some(mesh) = mesh.get(*entity) {
                        draw_mesh(
                            encoder,
                            effect,
                            self.skinning,
                            mesh_storage.get(mesh),
                            joints.get(*entity),
                            &tex_storage,
                            material.get(*entity),
                            &material_defaults,
                            rgba.get(*entity),
                            camera,
                            transform.get(*entity),
                            &ATTRIBUTES,
                            &TEXTURES,
                        );
                    }
                }
            }
        }
    }
}
