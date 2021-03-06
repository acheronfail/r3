use r3lib::{R3Command, WMCommand};
use xcb::{x, Xid};

use crate::wm_test;
use crate::x_test_runner::XTestCase;

wm_test!(maps_a_window, |t: XTestCase| {
    let w = t.open_window((0, 0, 30, 30));
    w.map();
    t.sync();

    // Check the window is mapped
    assert_eq!(w.rect(), (0, 0, 30, 30));
    assert_eq!(1, t.get_all_windows().len());

    // Check a frame was created
    let f = w.get_frame();
    assert_eq!(true, f.is_frame());
    assert_eq!(10, f.border_width());
});

wm_test!(kills_window_when_no_support_wm_delete_window, |t: XTestCase| {
    let w = t.open_window((0, 0, 30, 30));
    w.map();

    t.sync();
    assert_eq!(1, t.get_all_windows().len());

    t.command(R3Command::WM(WMCommand::CloseWindow));
    t.sync();
    assert_eq!(0, t.get_all_windows().len());
});

wm_test!(can_gracefully_kill_window, |t: XTestCase| {
    let w = t.open_window((0, 0, 300, 300));
    w.map();

    // Declare that we support WM_DELETE_WINDOW
    t.conn
        .send_and_check_request(&x::ChangeProperty {
            mode: x::PropMode::Replace,
            window: w.id,
            property: t.atoms.wm_protocols,
            r#type: x::ATOM_ATOM,
            data: &[t.atoms.wm_del_window],
        })
        .unwrap();

    t.sync();
    assert_eq!(1, t.get_all_windows().len());

    // Send the close window command
    t.command(R3Command::WM(WMCommand::CloseWindow));
    loop {
        // Make sure it didn't close unexpectedly (if wm didn't detect WM_DELETE_WINDOW support)
        assert_eq!(1, t.get_all_windows().len(), "Window closed unexpectedly early!");
        // When we receive the close message, just close our window
        if let xcb::Event::X(x::Event::ClientMessage(ev)) = t.conn.wait_for_event().unwrap() {
            if let x::ClientMessageData::Data32([atom, ..]) = ev.data() {
                if atom == t.atoms.wm_del_window.resource_id() {
                    w.close();
                    break;
                }
            }
        }
    }

    t.sync();
    assert_eq!(0, t.get_all_windows().len());
});
