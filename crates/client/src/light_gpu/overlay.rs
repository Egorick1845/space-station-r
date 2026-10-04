//! Оверлей мира по GPU-карте света (PORT_PLAN 1.1).
//!
//! В движке свет применяется к КАЖДОМУ спрайту умножением (`COLOR * LIGHT`,
//! инстанс шейдера с `lighting = true`). У нас ту же роль играет один квад
//! поверх мира с материалом-умножением: результат = кадр × карта света.
//! Умножение, а не альфа-смешивание, потому что карта света несёт ЦВЕТ лампы
//! (у чёрного оверлея с альфой цвет терялся бы, и его пришлось бы подмешивать
//! отдельным слоем свечения, как в CPU-пути).
//!
//! Квад растянут ровно на видимую область мира (`LightScene::viewport`), а карта
//! света считается по той же области, поэтому тексель карты ложится пиксель в
//! пиксель. UV меша идут снизу-вверх относительно мирового Y, а карта писалась
//! построчно от нижней границы — V переворачивается в шейдере.

use bevy::mesh::Mesh2d;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    SpecializedMeshPipelineError,
};
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, MeshMaterial2d};

use super::LightGpu;

/// Слой оверлея — выше ВСЕХ мировых спрайтов, ниже интерфейса: тот же слой
/// тьмы, что и у CPU-пути (`lighting::DARK_Z`).
pub const LIGHT_OVERLAY_Z: f32 = 2.01;

/// Материал оверлея: карта света конвейера.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct LightOverlayMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub light_map: Handle<Image>,
}

impl Material2d for LightOverlayMaterial {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/light_overlay.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    /// Мир × свет: `src.rgb * dst.rgb` (в движке это `COLOR * LIGHT` в шейдере
    /// спрайта). Альфу кадра не трогаем — иначе потемнел бы сам буфер кадра.
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(target) = descriptor
            .fragment
            .as_mut()
            .and_then(|fragment| fragment.targets.get_mut(0))
            .and_then(|target| target.as_mut())
        {
            target.blend = Some(BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::Dst,
                    dst_factor: BlendFactor::Zero,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent {
                    src_factor: BlendFactor::Zero,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                },
            });
        }
        Ok(())
    }
}

/// Квад оверлея (карта света подставляется при создании материала).
#[derive(Resource)]
pub struct LightOverlay {
    pub quad: Entity,
}

/// GPU-путь света включён, пока не попросили CPU (`SSR_LIGHT_CPU=1`).
pub fn gpu_lighting() -> bool {
    std::env::var_os("SSR_LIGHT_CPU").is_none()
}

/// Создаёт квад оверлея (один раз на старте). Размер задаётся единичным
/// прямоугольником и масштабом — масштаб выставляет `sync_light_overlay`.
pub fn setup_light_overlay(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LightOverlayMaterial>>,
    textures: Res<LightGpu>,
) {
    let material = materials.add(LightOverlayMaterial {
        light_map: textures.light_b.clone(),
    });
    let quad = commands
        .spawn((
            Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
            MeshMaterial2d(material.clone()),
            Transform::from_xyz(0.0, 0.0, LIGHT_OVERLAY_Z),
            Visibility::Hidden,
        ))
        .id();
    commands.insert_resource(LightOverlay { quad });
    tracing::info!(
        gpu = gpu_lighting(),
        "light gpu: квад оверлея создан (умножение кадра на карту света)"
    );
}

/// Каждый кадр: квад накрывает видимую область мира, слои CPU-пути скрыты при
/// работе GPU-пути (и наоборот — под `SSR_LIGHT_CPU=1`).
pub fn sync_light_overlay(
    overlay: Option<Res<LightOverlay>>,
    scene: Res<crate::lighting::LightScene>,
    light_map: Option<Res<crate::lighting::LightMap>>,
    mut transforms: Query<&mut Transform>,
    mut visibilities: Query<&mut Visibility>,
) {
    let gpu = gpu_lighting();
    let Some(overlay) = overlay else {
        return;
    };
    if let Ok(mut transform) = transforms.get_mut(overlay.quad) {
        transform.translation.x = scene.camera.0 + scene.viewport.0 * 0.5;
        transform.translation.y = scene.camera.1 + scene.viewport.1 * 0.5;
        transform.scale = Vec3::new(scene.viewport.0, scene.viewport.1, 1.0);
    }
    if let Ok(mut visibility) = visibilities.get_mut(overlay.quad) {
        let target = if gpu {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != target {
            *visibility = target;
        }
    }
    // Слои CPU-пути (тьма и оттенок) — наоборот.
    if let Some(light_map) = light_map {
        for sprite in light_map.sprites() {
            if let Ok(mut visibility) = visibilities.get_mut(sprite) {
                let target = if gpu {
                    Visibility::Hidden
                } else {
                    Visibility::Visible
                };
                if *visibility != target {
                    *visibility = target;
                }
            }
        }
    }
}
