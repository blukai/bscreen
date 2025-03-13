use crate::{
    gfx::{DrawBuffer, Rect, RectFill, Vec2},
    input::{CursorShape, Event, KeyboardEventKind, PointerEventKind, Scancode},
};

pub mod theme {
    use crate::gfx::Rgba8;

    pub const HANDLE_SIZE: f32 = 13.0;
    pub const HANDLE_BG: Rgba8 = Rgba8::new(255, 255, 255, 128);
    pub const OUTLINE: Rgba8 = Rgba8::new(48, 92, 222, 255);
    pub const OUTSIDE_BG: Rgba8 = Rgba8::new(0, 0, 0, 128);
}

#[derive(Debug)]
enum HandleType {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
    Inside,
}

impl HandleType {
    fn cursor_shape(&self) -> CursorShape {
        match self {
            Self::TopLeft => CursorShape::NwResize,
            Self::TopRight => CursorShape::NeResize,
            Self::BottomRight => CursorShape::SeResize,
            Self::BottomLeft => CursorShape::SwResize,
            Self::Inside => CursorShape::Move,
        }
    }
}

fn top_left_rect_handle(rect: &Rect) -> Rect {
    Rect::from_center_size(rect.top_left(), theme::HANDLE_SIZE)
}

fn top_right_rect_handle(rect: &Rect) -> Rect {
    Rect::from_center_size(rect.top_right(), theme::HANDLE_SIZE)
}

fn bottom_right_rect_handle(rect: &Rect) -> Rect {
    Rect::from_center_size(rect.bottom_right(), theme::HANDLE_SIZE)
}

fn bottom_left_rect_handle(rect: &Rect) -> Rect {
    Rect::from_center_size(rect.bottom_left(), theme::HANDLE_SIZE)
}

fn pointer_on_handle(rect: &Rect, pointer_position: &Vec2) -> Option<HandleType> {
    use HandleType::*;
    if top_left_rect_handle(rect).contains(pointer_position) {
        return Some(TopLeft);
    }
    if top_right_rect_handle(rect).contains(pointer_position) {
        return Some(TopRight);
    }
    if bottom_right_rect_handle(rect).contains(pointer_position) {
        return Some(BottomRight);
    }
    if bottom_left_rect_handle(rect).contains(pointer_position) {
        return Some(BottomLeft);
    }
    if rect.contains(pointer_position) {
        return Some(Inside);
    }
    None
}

#[derive(Debug, Default)]
pub struct Crop {
    pub view_rect: Option<Rect>,
    pub crop_rect: Option<Rect>,
    handle: Option<HandleType>,
    pub cursor: Option<CursorShape>,
}

impl Crop {
    pub fn update(&mut self, view_rect: Rect, event: &Event) -> bool {
        self.view_rect = Some(view_rect);
        let prev_crop_rect = self.crop_rect.clone();

        match event {
            Event::Pointer(pointer_event) => {
                match pointer_event.kind {
                    PointerEventKind::Press { .. } => {
                        if let Some(crop_rect) = self.crop_rect.as_ref() {
                            self.handle = pointer_on_handle(crop_rect, &pointer_event.position);
                            if self.handle.is_none() {
                                _ = self.crop_rect.take();
                            }
                        }
                        if self.crop_rect.is_none() {
                            self.crop_rect =
                                Some(Rect::from_center_size(pointer_event.position, 0.0));
                            self.handle = Some(HandleType::BottomRight);
                        }
                    }
                    PointerEventKind::Release { .. } => {
                        if let Some(crop_rect) = self.crop_rect.as_mut() {
                            *crop_rect = crop_rect.normalize().constrain_to(&view_rect);
                            let size = crop_rect.size();
                            if size.x < 1.0 || size.y < 1.0 {
                                _ = self.crop_rect.take();
                            }
                        }
                        _ = self.handle.take();
                    }
                    PointerEventKind::Motion { delta } => {
                        if let Some(crop_rect) = self.crop_rect.as_mut() {
                            if let Some(handle) = self.handle.as_ref() {
                                match handle {
                                    HandleType::TopLeft => {
                                        crop_rect.set_top_left(crop_rect.top_left() + delta)
                                    }
                                    HandleType::TopRight => {
                                        crop_rect.set_top_right(crop_rect.top_right() + delta)
                                    }
                                    HandleType::BottomRight => {
                                        crop_rect.set_bottom_right(crop_rect.bottom_right() + delta)
                                    }
                                    HandleType::BottomLeft => {
                                        crop_rect.set_bottom_left(crop_rect.bottom_left() + delta)
                                    }
                                    HandleType::Inside => *crop_rect = crop_rect.translate(&delta),
                                }
                            }
                        }
                    }
                    _ => {}
                }

                self.cursor = Some(CursorShape::Crosshair);
                if let Some(crop_rect) = self.crop_rect.as_ref() {
                    if let Some(handle) =
                        pointer_on_handle(&crop_rect.normalize(), &pointer_event.position)
                    {
                        self.cursor = Some(handle.cursor_shape());
                    }
                }
            }
            Event::Keyboard(keyboard_event) => match keyboard_event.kind {
                KeyboardEventKind::Press {
                    scancode: Scancode::A,
                } if keyboard_event.mods.ctrl => {
                    self.crop_rect = Some(view_rect);

                    self.cursor = Some(CursorShape::Move);
                }
                _ => {}
            },
        };

        !prev_crop_rect.eq(&self.crop_rect)
    }

    pub fn draw(&self, draw_buffer: &mut DrawBuffer) {
        let Some(view_rect) = self.view_rect.as_ref() else {
            return;
        };
        let crop_rect = if let Some(crop_rect) = self.crop_rect.as_ref() {
            crop_rect.normalize().constrain_to(&view_rect)
        } else {
            return;
        };

        // darkening rects
        // ----

        {
            let fill = RectFill::Color(theme::OUTSIDE_BG);

            // horizontal top, full width
            draw_buffer.push_rect_filled(
                Rect::new(view_rect.min, Vec2::new(view_rect.max.x, crop_rect.min.y)),
                fill,
            );
            // horizontal bottom, full width
            draw_buffer.push_rect_filled(
                Rect::new(Vec2::new(view_rect.min.x, crop_rect.max.y), view_rect.max),
                fill,
            );
            // vertical left, between horizontal
            draw_buffer.push_rect_filled(
                Rect::new(
                    Vec2::new(view_rect.min.x, crop_rect.min.y),
                    Vec2::new(crop_rect.min.x, crop_rect.max.y),
                ),
                fill,
            );
            // vertical right, between horizontal
            draw_buffer.push_rect_filled(
                Rect::new(
                    Vec2::new(crop_rect.max.x, crop_rect.min.y),
                    Vec2::new(view_rect.max.x, crop_rect.max.y),
                ),
                fill,
            );
        }

        // outline
        // ----

        let outline_width = 1.0;
        let outline_color = theme::OUTLINE;

        {
            draw_buffer.push_rect_outlined(crop_rect, outline_width, outline_color);
        }

        // corner handles
        // ----

        {
            let fill = RectFill::Color(theme::HANDLE_BG);

            draw_buffer.push_rect(
                top_left_rect_handle(&crop_rect),
                Some(fill),
                Some(outline_width),
                Some(outline_color),
            );
            draw_buffer.push_rect(
                top_right_rect_handle(&crop_rect),
                Some(fill),
                Some(outline_width),
                Some(outline_color),
            );
            draw_buffer.push_rect(
                bottom_right_rect_handle(&crop_rect),
                Some(fill),
                Some(outline_width),
                Some(outline_color),
            );
            draw_buffer.push_rect(
                bottom_left_rect_handle(&crop_rect),
                Some(fill),
                Some(outline_width),
                Some(outline_color),
            );
        }
    }
}
