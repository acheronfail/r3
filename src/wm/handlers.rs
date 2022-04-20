use std::cmp;

use xcb::x::{
    self, ButtonPressEvent, ConfigureRequestEvent, EnterNotifyEvent, ExposeEvent, FocusInEvent, FocusOutEvent,
    KeyPressEvent, LeaveNotifyEvent, MapRequestEvent, MotionNotifyEvent, PropertyNotifyEvent, UnmapNotifyEvent,
};

use super::{DragType, QuitReason, WindowManager};
use crate::{point::Point, window_geometry::Quadrant, ret_ok_if_none};

impl WindowManager {
    /**
     * X Events
     */

    pub(super) fn on_configure_request(&self, ev: ConfigureRequestEvent) -> xcb::Result<()> {
        let window = ev.window();
        let value_list = [
            x::ConfigWindow::X(ev.x() as i32),
            x::ConfigWindow::Y(ev.y() as i32),
            x::ConfigWindow::Width(ev.width() as u32),
            x::ConfigWindow::Height(ev.height() as u32),
            x::ConfigWindow::BorderWidth(ev.border_width() as u32),
            // FIXME: this crashes it when ev.sibling() returns 0
            // x::ConfigWindow::Sibling(ev.sibling()),
            x::ConfigWindow::StackMode(ev.stack_mode()),
        ];

        // If we've already framed this window, also update the frame
        if let Some(frame_id) = self.framed_clients.get_by_left(&window) {
            self.conn.send_and_check_request(&x::ConfigureWindow {
                window: *frame_id,
                value_list: &value_list,
            })?;
        }

        // Pass request straight through to the X server for window
        self.conn.send_and_check_request(&x::ConfigureWindow {
            window,
            value_list: &value_list,
        })?;

        Ok(())
    }

    pub(super) fn on_map_request(&mut self, ev: MapRequestEvent) -> xcb::Result<()> {
        let window = ev.window();
        // First, we re-parent it with a frame
        let frame = self.frame_window(window, false)?;

        // Then, we actually map the window
        self.conn.send_and_check_request(&x::MapWindow { window })?;

        // Focus the newly mapped window or frame (if one was created)
        self.focused_window = frame.or(Some(window));

        Ok(())
    }

    pub(super) fn on_unmap_notify(&mut self, ev: UnmapNotifyEvent) -> xcb::Result<()> {
        // Any windows existing before we started that are framed in `App::reparent_existing_windows`
        // trigger an UnmapNotify event when they're re-parented. We just ignore these events here.
        if ev.event() == self.get_root_window()? {
            return Ok(());
        }

        self.unframe_window(ev.window())?;
        Ok(())
    }

    /**
     * Key Events
     */

    // TODO: remove hardcoded values when configuration is available
    pub(super) fn on_key_press(&mut self, ev: KeyPressEvent) -> xcb::Result<()> {
        // CTRL + SHIFT + Q - kill window manager
        // TODO: this has to be fired on a window
        if ev.state().contains(x::KeyButMask::CONTROL | x::KeyButMask::SHIFT) && ev.detail() == 0x18 {
            self.quit_reason = Some(QuitReason::UserQuit);
            return Ok(());
        }

        // TODO: we choose focused window by cursor right now, but that's not right (should be whichever has focus, or is active, etc)
        if let Some(window) = self.window_at_pos(ev.root(), (ev.root_x(), ev.root_y()).into())? {
            // CTRL + Q (on qwerty) - kill window
            if ev.state().contains(x::KeyButMask::CONTROL) && ev.detail() == 0x18 {
                self.kill_window(window)?;
            }
        }

        Ok(())
    }

    pub(super) fn on_key_release(&self, _ev: KeyPressEvent) -> xcb::Result<()> {
        Ok(())
    }

    /**
     * Mouse Events
     */

    pub(super) fn on_button_press(&mut self, ev: ButtonPressEvent) -> xcb::Result<()> {
        let target = ev.event();
        let (window, frame) = ret_ok_if_none!(self.get_frame_and_window(target));

        // Start a drag if Ctrl is pressed
        // TODO: configurable modifier
        if ev.state().contains(x::KeyButMask::CONTROL) || target == frame {
            self.drag_start = Some((ev.root_x(), ev.root_y()).into());
            self.drag_start_frame_rect = Some(self.get_window_rect(frame)?);
        }

        // Focus and raise window
        self.focused_window = Some(window);
        self.conn.send_and_check_request(&x::ConfigureWindow {
            window: frame,
            value_list: &[x::ConfigWindow::StackMode(x::StackMode::Above)],
        })?;

        Ok(())
    }

    // TODO: remove hardcoded values when configuration is available
    pub(super) fn on_motion_notify(&mut self, ev: MotionNotifyEvent) -> xcb::Result<()> {
        let target = ev.event();
        let (window, _) = ret_ok_if_none!(self.get_frame_and_window(target));

        let drag_start = ret_ok_if_none!(self.drag_start);
        let drag_start_frame_rect = ret_ok_if_none!(self.drag_start_frame_rect);

        let delta = Point::new(ev.root_x(), ev.root_y()) - drag_start;
        let drag_type = ret_ok_if_none!(if ev.state().contains(x::KeyButMask::BUTTON1) {
            Some(DragType::Move)
        } else if ev.state().contains(x::KeyButMask::BUTTON3) {
            Some(DragType::Resize)
        } else {
            None
        });

        match drag_type {
            DragType::Move => self.move_window(
                window,
                (drag_start_frame_rect.x + delta.x, drag_start_frame_rect.y + delta.y).into(),
            )?,
            DragType::Resize => self.resize_window(
                window,
                match ret_ok_if_none!(drag_start_frame_rect.quadrant(&drag_start)) {
                    Quadrant::TopLeft => (
                        drag_start_frame_rect.x + delta.x,
                        drag_start_frame_rect.y + delta.y,
                        cmp::max(1, drag_start_frame_rect.w as i32 - delta.x as i32) as u16,
                        cmp::max(1, drag_start_frame_rect.h as i32 - delta.y as i32) as u16,
                    ),
                    Quadrant::TopRight => (
                        drag_start_frame_rect.x,
                        drag_start_frame_rect.y + delta.y,
                        cmp::max(1, drag_start_frame_rect.w as i32 + delta.x as i32) as u16,
                        cmp::max(1, drag_start_frame_rect.h as i32 - delta.y as i32) as u16,
                    ),
                    Quadrant::BottomLeft => (
                        drag_start_frame_rect.x + delta.x,
                        drag_start_frame_rect.y,
                        cmp::max(1, drag_start_frame_rect.w as i32 - delta.x as i32) as u16,
                        cmp::max(1, drag_start_frame_rect.h as i32 + delta.y as i32) as u16,
                    ),
                    Quadrant::BottomRight => (
                        drag_start_frame_rect.x,
                        drag_start_frame_rect.y,
                        cmp::max(1, drag_start_frame_rect.w as i32 + delta.x as i32) as u16,
                        cmp::max(1, drag_start_frame_rect.h as i32 + delta.y as i32) as u16,
                    ),
                }
                .into(),
            )?,
        }

        Ok(())
    }

    pub(super) fn on_button_release(&mut self, _ev: ButtonPressEvent) -> xcb::Result<()> {
        self.drag_start = None;
        self.drag_start_frame_rect = None;
        Ok(())
    }

    /**
     * Window Events
     */

    pub(super) fn on_enter_notify(&mut self, ev: EnterNotifyEvent) -> xcb::Result<()> {
        if self.config.focus_follows_mouse {
            let target = ev.event();
            self.focused_window = Some(*self.framed_clients.get_by_right(&target).unwrap_or(&target));
        }

        Ok(())
    }

    pub(super) fn on_leave_notify(&self, _ev: LeaveNotifyEvent) -> xcb::Result<()> {
        Ok(())
    }

    pub(super) fn on_expose(&self, _ev: ExposeEvent) -> xcb::Result<()> {
        Ok(())
    }

    pub(super) fn on_focus_in(&self, _ev: FocusInEvent) -> xcb::Result<()> {
        Ok(())
    }

    pub(super) fn on_focus_out(&self, _ev: FocusOutEvent) -> xcb::Result<()> {
        Ok(())
    }

    pub(super) fn on_property_notify(&self, _ev: PropertyNotifyEvent) -> xcb::Result<()> {
        Ok(())
    }
}
