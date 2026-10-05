//! Перетаскивание окон UI и запоминание их позиций.
//!
//! Любая панель с [`WindowDrag`] тянется ЛКМ за любую точку, кроме кнопок
//! внутри (кнопки блокируют фокус и обрабатывают клик сами). Позиция
//! запоминается в [`WindowPositions`] и применяется при перерисовке панели.

use bevy::prelude::*;

/// Вид окна — ключ для запоминания позиции.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Inventory,
    /// Окно персонажа со слотами одежды (`InventoryGui`) — своя позиция, иначе
    /// перетаскивание одного окна уводило и второе.
    Character,
    Hands,
    Container,
    /// Крафт-меню (`ConstructionMenu` в сборке).
    Craft,
}

/// Позиции окон (левый верхний угол, логические px); None — позиция по умолчанию.
#[derive(Resource, Default)]
pub struct WindowPositions {
    inventory: Option<Vec2>,
    character: Option<Vec2>,
    hands: Option<Vec2>,
    container: Option<Vec2>,
    craft: Option<Vec2>,
}

impl WindowPositions {
    pub fn get(&self, kind: WindowKind) -> Option<Vec2> {
        match kind {
            WindowKind::Inventory => self.inventory,
            WindowKind::Character => self.character,
            WindowKind::Hands => self.hands,
            WindowKind::Container => self.container,
            WindowKind::Craft => self.craft,
        }
    }

    pub fn set(&mut self, kind: WindowKind, pos: Vec2) {
        match kind {
            WindowKind::Inventory => self.inventory = Some(pos),
            WindowKind::Character => self.character = Some(pos),
            WindowKind::Hands => self.hands = Some(pos),
            WindowKind::Container => self.container = Some(pos),
            WindowKind::Craft => self.craft = Some(pos),
        }
    }
}

/// Панель, которую можно тащить мышью.
#[derive(Component, Default)]
pub struct WindowDrag {
    grab: Option<Vec2>,
    last: Option<Vec2>,
}

/// Ставит сохранённую позицию окна (если игрок уже двигал его).
pub fn apply_saved_position(kind: WindowKind, node: &mut Node, positions: &WindowPositions) {
    if let Some(pos) = positions.get(kind) {
        node.left = px(pos.x);
        node.top = px(pos.y);
        node.bottom = Val::Auto;
        node.right = Val::Auto;
    }
}

/// Перетаскивание: ЛКМ, начатая над панелью, тянет её за курсором.
pub fn drag_windows(
    mut commands: Commands,
    windows: Query<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut positions: ResMut<WindowPositions>,
    mut panels: Query<(
        Entity,
        &WindowKind,
        &Interaction,
        &mut WindowDrag,
        &mut Node,
        &ComputedNode,
        &GlobalTransform,
    )>,
    mut top_z: Local<i32>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let held = mouse.pressed(MouseButton::Left);
    for (entity, kind, interaction, mut drag, mut node, computed, transform) in panels.iter_mut() {
        match (held, drag.grab) {
            // Начало перетаскивания: ЛКМ легла на саму панель.
            (true, None) => {
                if *interaction == Interaction::Pressed {
                    let size = computed.size() * computed.inverse_scale_factor();
                    let top_left = transform.translation().truncate() - size / 2.0;
                    drag.grab = Some(cursor - top_left);
                    *top_z += 1;
                    commands.entity(entity).insert(GlobalZIndex(*top_z));
                }
            }
            // Тянем за курсором.
            (true, Some(grab)) => {
                let pos = cursor - grab;
                node.left = px(pos.x);
                node.top = px(pos.y);
                node.bottom = Val::Auto;
                node.right = Val::Auto;
                drag.last = Some(pos);
            }
            // Отпустили — запоминаем позицию.
            (false, Some(_)) => {
                drag.grab = None;
                if let Some(pos) = drag.last.take() {
                    positions.set(*kind, pos);
                }
            }
            (false, None) => {}
        }
    }
}
