#![windows_subsystem = "windows"]
#![allow(static_mut_refs, dead_code)]

use std::collections::VecDeque;
use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::time::Instant;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WINDOW_WIDTH: i32 = 460;
const WINDOW_HEIGHT: i32 = 530;

const CF_UNICODETEXT: u32 = 13;
const WM_MOUSELEAVE: u32 = 0x02A3;

// Button identifiers
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum ButtonId {
    PickScreen,
    CopyHex,
    CopyRgb,
    CopyHsl,
    AutoCopyToggle,
    RecentSwatch(usize),
}

// Global application state
struct AppState {
    current_color: (u8, u8, u8),
    recent_colors: VecDeque<(u8, u8, u8)>,
    copied_at: Option<Instant>,
    copied_text: String,
    auto_copy: bool,
    hovered_button: Option<ButtonId>,
    pressed_button: Option<ButtonId>,
    status_message: String,
    hfont_title: HFONT,
    hfont_hex: HFONT,
    hfont_medium: HFONT,
    hfont_regular: HFONT,
    hfont_small: HFONT,
    app_icon: HICON,
}

static mut STATE: Option<AppState> = None;
static mut MAIN_HWND: HWND = null_mut();
static mut OVERLAY_HWND: HWND = null_mut();

// Overlay snapshot and coordinates
struct OverlayData {
    vx: i32,
    vy: i32,
    vw: i32,
    vh: i32,
    snapshot_dc: HDC,
    snapshot_bmp: HBITMAP,
    old_bmp: HGDIOBJ,
    cursor_pt: POINT,
    current_col: (u8, u8, u8),
}

static mut OVERLAY: Option<OverlayData> = None;

// Helper: wide string
fn to_wstring(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// Convert RGB to HEX
fn rgb_to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

// Convert RGB to rgb() string
fn rgb_to_string(r: u8, g: u8, b: u8) -> String {
    format!("rgb({}, {}, {})", r, g, b)
}

// Convert RGB to HSL string
fn rgb_to_hsl_string(r: u8, g: u8, b: u8) -> String {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;

    let max = rf.max(gf.max(bf));
    let min = rf.min(gf.min(bf));
    let delta = max - min;

    let l = (max + min) / 2.0;

    let (h, s) = if delta == 0.0 {
        (0.0, 0.0)
    } else {
        let s = if l < 0.5 {
            delta / (max + min)
        } else {
            delta / (2.0 - max - min)
        };

        let mut h = if rf == max {
            (gf - bf) / delta
        } else if gf == max {
            2.0 + (bf - rf) / delta
        } else {
            4.0 + (rf - gf) / delta
        };

        h *= 60.0;
        if h < 0.0 {
            h += 360.0;
        }
        (h, s)
    };

    format!("hsl({:.0}°, {:.0}%, {:.0}%)", h, s * 100.0, l * 100.0)
}

// Copy text to Windows Clipboard
fn copy_to_clipboard(hwnd: HWND, text: &str) -> bool {
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return false;
        }
        EmptyClipboard();

        let wide = to_wstring(text);
        let bytes_len = wide.len() * std::mem::size_of::<u16>();

        let h_mem = GlobalAlloc(GMEM_MOVEABLE, bytes_len);
        if !h_mem.is_null() {
            let ptr = GlobalLock(h_mem) as *mut u16;
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                GlobalUnlock(h_mem);
                SetClipboardData(CF_UNICODETEXT, h_mem as _);
            }
        }

        CloseClipboard();
        true
    }
}

// Create GDI font helper
fn create_font(name: &str, size: i32, bold: bool, monospace: bool) -> HFONT {
    unsafe {
        let wide_name = to_wstring(name);
        CreateFontW(
            size,
            0,
            0,
            0,
            if bold { FW_BOLD as i32 } else { FW_NORMAL as i32 },
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            if monospace { FIXED_PITCH as u32 } else { VARIABLE_PITCH as u32 },
            wide_name.as_ptr(),
        )
    }
}

// Color constants (COLORREF = 0x00BBGGRR)
fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

const COLOR_BG: u32 = 0x001B1816;          // #16181B dark slate
const COLOR_CARD_BG: u32 = 0x0026211F;     // #1F2126 card background
const COLOR_CARD_BORDER: u32 = 0x003D3734; // #34373D border
const COLOR_TEXT_WHITE: u32 = 0x00F8FAFC;  // #FCFAF8
const COLOR_TEXT_MUTED: u32 = 0x00948F8B;  // #8B8F94
const COLOR_ACCENT: u32 = 0x00E5464F;      // Indigo/Purple #4F46E5 in COLORREF
const COLOR_ACCENT_HOVER: u32 = 0x00F15E63;// #635EF1
const COLOR_SUCCESS: u32 = 0x005DBA34;     // Emerald #34BA5D
const COLOR_DANGER: u32 = 0x004444EF;      // Red accent

// Draw rounded rect with border
unsafe fn draw_rounded_card(hdc: HDC, rect: &RECT, bg_color: u32, border_color: u32, radius: i32) {
    let brush = CreateSolidBrush(bg_color);
    let pen = CreatePen(PS_SOLID as i32, 1, border_color);
    let old_brush = SelectObject(hdc, brush);
    let old_pen = SelectObject(hdc, pen);

    RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);

    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    DeleteObject(brush);
    DeleteObject(pen);
}

// Main Window Procedure
unsafe extern "system" fn main_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            // Enable immersive dark mode on Windows 10/11
            let dark_mode: i32 = 1;
            DwmSetWindowAttribute(
                hwnd,
                20, // DWMWA_USE_IMMERSIVE_DARK_MODE
                &dark_mode as *const _ as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );

            // Set up timer for copied badge fadeout
            SetTimer(hwnd, 1, 100, None);
            0
        }

        WM_TIMER => {
            if let Some(state) = STATE.as_mut() {
                if let Some(at) = state.copied_at {
                    if at.elapsed().as_millis() > 2000 {
                        state.copied_at = None;
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
            }
            0
        }

        WM_KEYDOWN => {
            if wparam == VK_SPACE as usize || wparam == VK_RETURN as usize {
                start_screen_picker();
            } else if wparam == 0x43 { // 'C' key
                let shift = (GetKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
                if shift {
                    if let Some(state) = STATE.as_mut() {
                        let hex = rgb_to_hex(state.current_color.0, state.current_color.1, state.current_color.2);
                        copy_to_clipboard(hwnd, &hex);
                        state.copied_at = Some(Instant::now());
                        state.copied_text = "HEX".to_string();
                        state.status_message = format!("Copied {} to clipboard!", hex);
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
            }
            0
        }

        WM_MOUSEMOVE => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

            let prev_hover = STATE.as_ref().and_then(|s| s.hovered_button);
            let new_hover = hit_test(x, y);

            if prev_hover != new_hover {
                if let Some(state) = STATE.as_mut() {
                    state.hovered_button = new_hover;
                }

                // Track mouse leave
                let mut tme: TRACKMOUSEEVENT = zeroed();
                tme.cbSize = std::mem::size_of::<TRACKMOUSEEVENT>() as u32;
                tme.dwFlags = TME_LEAVE;
                tme.hwndTrack = hwnd;
                TrackMouseEvent(&mut tme);

                InvalidateRect(hwnd, null(), 0);
            }
            0
        }

        WM_MOUSELEAVE => {
            if let Some(state) = STATE.as_mut() {
                state.hovered_button = None;
                state.pressed_button = None;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }

        WM_LBUTTONDOWN => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

            if let Some(btn) = hit_test(x, y) {
                if let Some(state) = STATE.as_mut() {
                    state.pressed_button = Some(btn);
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }

        WM_LBUTTONUP => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

            if let Some(state) = STATE.as_mut() {
                let pressed = state.pressed_button;
                state.pressed_button = None;

                if let Some(btn) = pressed {
                    if hit_test(x, y) == Some(btn) {
                        handle_button_click(hwnd, btn);
                    }
                }
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }

        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut client_rect: RECT = zeroed();
            GetClientRect(hwnd, &mut client_rect);

            // Double buffered painting to avoid flicker
            let mem_dc = CreateCompatibleDC(hdc);
            let mem_bmp = CreateCompatibleBitmap(hdc, client_rect.right, client_rect.bottom);
            let old_bmp = SelectObject(mem_dc, mem_bmp);

            draw_main_interface(mem_dc, &client_rect);

            BitBlt(
                hdc,
                0,
                0,
                client_rect.right,
                client_rect.bottom,
                mem_dc,
                0,
                0,
                SRCCOPY,
            );

            SelectObject(mem_dc, old_bmp);
            DeleteObject(mem_bmp);
            DeleteDC(mem_dc);

            EndPaint(hwnd, &ps);
            0
        }

        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// Determine which button is at coordinate (x, y)
fn hit_test(x: i32, y: i32) -> Option<ButtonId> {
    // 1. Pick button: rect (20, 68, 440, 114)
    if x >= 20 && x <= 440 && y >= 68 && y <= 114 {
        return Some(ButtonId::PickScreen);
    }

    // 2. Copy HEX button: rect (150, 194, 300, 232)
    if x >= 150 && x <= 300 && y >= 194 && y <= 232 {
        return Some(ButtonId::CopyHex);
    }

    // 3. Copy RGB button: rect (356, 258, 424, 288)
    if x >= 356 && x <= 424 && y >= 258 && y <= 288 {
        return Some(ButtonId::CopyRgb);
    }

    // 4. Copy HSL button: rect (356, 298, 424, 328)
    if x >= 356 && x <= 424 && y >= 298 && y <= 328 {
        return Some(ButtonId::CopyHsl);
    }

    // 5. Auto-copy checkbox: rect (20, 348, 440, 374)
    if x >= 20 && x <= 440 && y >= 348 && y <= 374 {
        return Some(ButtonId::AutoCopyToggle);
    }

    // 6. Recent swatches: rect starts around y: 418
    if y >= 418 && y <= 456 {
        for i in 0..10 {
            let sx = 22 + (i as i32 * 42);
            if x >= sx && x <= sx + 34 {
                return Some(ButtonId::RecentSwatch(i));
            }
        }
    }

    None
}

// Handle UI button action
unsafe fn handle_button_click(hwnd: HWND, btn: ButtonId) {
    let state = match STATE.as_mut() {
        Some(s) => s,
        None => return,
    };

    match btn {
        ButtonId::PickScreen => {
            start_screen_picker();
        }

        ButtonId::CopyHex => {
            let hex = rgb_to_hex(state.current_color.0, state.current_color.1, state.current_color.2);
            copy_to_clipboard(hwnd, &hex);
            state.copied_at = Some(Instant::now());
            state.copied_text = "HEX".to_string();
            state.status_message = format!("Copied {} to clipboard!", hex);
        }

        ButtonId::CopyRgb => {
            let rgb_str = rgb_to_string(state.current_color.0, state.current_color.1, state.current_color.2);
            copy_to_clipboard(hwnd, &rgb_str);
            state.copied_at = Some(Instant::now());
            state.copied_text = "RGB".to_string();
            state.status_message = format!("Copied {} to clipboard!", rgb_str);
        }

        ButtonId::CopyHsl => {
            let hsl_str = rgb_to_hsl_string(state.current_color.0, state.current_color.1, state.current_color.2);
            copy_to_clipboard(hwnd, &hsl_str);
            state.copied_at = Some(Instant::now());
            state.copied_text = "HSL".to_string();
            state.status_message = format!("Copied {} to clipboard!", hsl_str);
        }

        ButtonId::AutoCopyToggle => {
            state.auto_copy = !state.auto_copy;
            state.status_message = if state.auto_copy {
                "Auto-copy enabled: Screen clicks copy HEX automatically.".to_string()
            } else {
                "Auto-copy disabled.".to_string()
            };
        }

        ButtonId::RecentSwatch(idx) => {
            if idx < state.recent_colors.len() {
                let col = state.recent_colors[idx];
                state.current_color = col;
                let hex = rgb_to_hex(col.0, col.1, col.2);
                if state.auto_copy {
                    copy_to_clipboard(hwnd, &hex);
                    state.copied_at = Some(Instant::now());
                    state.copied_text = "HEX".to_string();
                    state.status_message = format!("Restored & copied {} to clipboard!", hex);
                } else {
                    state.status_message = format!("Restored {} from palette.", hex);
                }
            }
        }
    }
}

// Render the main window UI
unsafe fn draw_main_interface(hdc: HDC, rc: &RECT) {
    let state = match STATE.as_ref() {
        Some(s) => s,
        None => return,
    };

    // 1. Background fill
    let bg_brush = CreateSolidBrush(COLOR_BG);
    FillRect(hdc, rc, bg_brush);
    DeleteObject(bg_brush);

    SetBkMode(hdc, TRANSPARENT as i32);

    // 2. Header
    // Draw 3D palette icon
    if !state.app_icon.is_null() {
        DrawIconEx(hdc, 20, 16, state.app_icon, 40, 40, 0, null_mut(), DI_NORMAL);
    }

    SelectObject(hdc, state.hfont_title);
    SetTextColor(hdc, COLOR_TEXT_WHITE);
    let title_w = to_wstring("Screen Color Picker");
    TextOutW(hdc, 68, 14, title_w.as_ptr(), (title_w.len() - 1) as i32);

    SelectObject(hdc, state.hfont_small);
    SetTextColor(hdc, COLOR_TEXT_MUTED);
    let sub_w = to_wstring("Instant pixel eyedropper & HEX palette");
    TextOutW(hdc, 70, 38, sub_w.as_ptr(), (sub_w.len() - 1) as i32);

    // 3. Primary "Pick Color" Button
    let pick_hover = state.hovered_button == Some(ButtonId::PickScreen);
    let pick_pressed = state.pressed_button == Some(ButtonId::PickScreen);
    let pick_rc = RECT { left: 20, top: 68, right: 440, bottom: 114 };

    let btn_bg = if pick_pressed {
        rgb(67, 56, 202) // #4338CA
    } else if pick_hover {
        COLOR_ACCENT_HOVER
    } else {
        COLOR_ACCENT
    };
    draw_rounded_card(hdc, &pick_rc, btn_bg, btn_bg, 10);

    SelectObject(hdc, state.hfont_medium);
    SetTextColor(hdc, COLOR_TEXT_WHITE);
    let pick_text = to_wstring("🎯  Pick Color From Screen  (Click or Press Space)");
    let mut draw_rc = pick_rc;
    DrawTextW(hdc, pick_text.as_ptr(), (pick_text.len() - 1) as i32, &mut draw_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    // 4. Current Color Card
    let card1_rc = RECT { left: 20, top: 126, right: 440, bottom: 242 };
    draw_rounded_card(hdc, &card1_rc, COLOR_CARD_BG, COLOR_CARD_BORDER, 12);

    // Color Swatch
    let swatch_rc = RECT { left: 34, top: 138, right: 136, bottom: 230 };
    let cur_colref = rgb(state.current_color.0, state.current_color.1, state.current_color.2);
    draw_rounded_card(hdc, &swatch_rc, cur_colref, COLOR_CARD_BORDER, 10);

    // Inner subtle white highlight
    let inner_pen = CreatePen(PS_SOLID as i32, 1, rgb(255, 255, 255));
    let old_pen = SelectObject(hdc, inner_pen);
    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH as i32));
    RoundRect(hdc, swatch_rc.left + 2, swatch_rc.top + 2, swatch_rc.right - 2, swatch_rc.bottom - 2, 8, 8);
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    DeleteObject(inner_pen);

    // HEX Label
    SelectObject(hdc, state.hfont_small);
    SetTextColor(hdc, COLOR_TEXT_MUTED);
    let hex_lbl = to_wstring("HEX COLOR");
    TextOutW(hdc, 150, 142, hex_lbl.as_ptr(), (hex_lbl.len() - 1) as i32);

    // Large Monospace HEX value
    SelectObject(hdc, state.hfont_hex);
    SetTextColor(hdc, COLOR_TEXT_WHITE);
    let hex_val = to_wstring(&rgb_to_hex(state.current_color.0, state.current_color.1, state.current_color.2));
    TextOutW(hdc, 150, 158, hex_val.as_ptr(), (hex_val.len() - 1) as i32);

    // Primary Copy Button
    let copy_hover = state.hovered_button == Some(ButtonId::CopyHex);
    let is_copied_hex = state.copied_at.is_some() && state.copied_text == "HEX";

    let copy_btn_rc = RECT { left: 150, top: 194, right: 300, bottom: 232 };
    let copy_bg = if is_copied_hex {
        COLOR_SUCCESS
    } else if copy_hover {
        rgb(55, 65, 81)
    } else {
        rgb(39, 45, 56)
    };
    let copy_border = if is_copied_hex {
        COLOR_SUCCESS
    } else if copy_hover {
        COLOR_ACCENT_HOVER
    } else {
        COLOR_CARD_BORDER
    };
    draw_rounded_card(hdc, &copy_btn_rc, copy_bg, copy_border, 8);

    SelectObject(hdc, state.hfont_medium);
    SetTextColor(hdc, COLOR_TEXT_WHITE);
    let copy_btn_txt = if is_copied_hex {
        to_wstring("✓  Copied!")
    } else {
        to_wstring("📋  Copy HEX")
    };
    let mut btn_text_rc = copy_btn_rc;
    DrawTextW(hdc, copy_btn_txt.as_ptr(), (copy_btn_txt.len() - 1) as i32, &mut btn_text_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    // 5. RGB / HSL Details Card
    let card2_rc = RECT { left: 20, top: 250, right: 440, bottom: 338 };
    draw_rounded_card(hdc, &card2_rc, COLOR_CARD_BG, COLOR_CARD_BORDER, 12);

    // RGB Row
    SelectObject(hdc, state.hfont_small);
    SetTextColor(hdc, COLOR_TEXT_MUTED);
    let rgb_lbl = to_wstring("RGB:");
    TextOutW(hdc, 34, 264, rgb_lbl.as_ptr(), (rgb_lbl.len() - 1) as i32);

    SelectObject(hdc, state.hfont_regular);
    SetTextColor(hdc, COLOR_TEXT_WHITE);
    let rgb_val = to_wstring(&rgb_to_string(state.current_color.0, state.current_color.1, state.current_color.2));
    TextOutW(hdc, 74, 262, rgb_val.as_ptr(), (rgb_val.len() - 1) as i32);

    // Mini copy button for RGB
    let is_copied_rgb = state.copied_at.is_some() && state.copied_text == "RGB";
    let rgb_btn_rc = RECT { left: 356, top: 258, right: 424, bottom: 288 };
    let rgb_btn_bg = if is_copied_rgb { COLOR_SUCCESS } else if state.hovered_button == Some(ButtonId::CopyRgb) { rgb(55, 65, 81) } else { rgb(39, 45, 56) };
    draw_rounded_card(hdc, &rgb_btn_rc, rgb_btn_bg, COLOR_CARD_BORDER, 6);
    SelectObject(hdc, state.hfont_small);
    SetTextColor(hdc, COLOR_TEXT_WHITE);
    let rgb_btn_txt = if is_copied_rgb { to_wstring("Copied") } else { to_wstring("Copy") };
    let mut rgb_draw_rc = rgb_btn_rc;
    DrawTextW(hdc, rgb_btn_txt.as_ptr(), (rgb_btn_txt.len() - 1) as i32, &mut rgb_draw_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    // HSL Row
    SelectObject(hdc, state.hfont_small);
    SetTextColor(hdc, COLOR_TEXT_MUTED);
    let hsl_lbl = to_wstring("HSL:");
    TextOutW(hdc, 34, 302, hsl_lbl.as_ptr(), (hsl_lbl.len() - 1) as i32);

    SelectObject(hdc, state.hfont_regular);
    SetTextColor(hdc, COLOR_TEXT_WHITE);
    let hsl_val = to_wstring(&rgb_to_hsl_string(state.current_color.0, state.current_color.1, state.current_color.2));
    TextOutW(hdc, 74, 300, hsl_val.as_ptr(), (hsl_val.len() - 1) as i32);

    // Mini copy button for HSL
    let is_copied_hsl = state.copied_at.is_some() && state.copied_text == "HSL";
    let hsl_btn_rc = RECT { left: 356, top: 298, right: 424, bottom: 328 };
    let hsl_btn_bg = if is_copied_hsl { COLOR_SUCCESS } else if state.hovered_button == Some(ButtonId::CopyHsl) { rgb(55, 65, 81) } else { rgb(39, 45, 56) };
    draw_rounded_card(hdc, &hsl_btn_rc, hsl_btn_bg, COLOR_CARD_BORDER, 6);
    SelectObject(hdc, state.hfont_small);
    SetTextColor(hdc, COLOR_TEXT_WHITE);
    let hsl_btn_txt = if is_copied_hsl { to_wstring("Copied") } else { to_wstring("Copy") };
    let mut hsl_draw_rc = hsl_btn_rc;
    DrawTextW(hdc, hsl_btn_txt.as_ptr(), (hsl_btn_txt.len() - 1) as i32, &mut hsl_draw_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    // 6. Auto-copy Toggle Checkbox
    let check_hover = state.hovered_button == Some(ButtonId::AutoCopyToggle);
    let check_box_rc = RECT { left: 24, top: 352, right: 40, bottom: 368 };
    let check_bg = if state.auto_copy { COLOR_ACCENT } else { rgb(39, 45, 56) };
    draw_rounded_card(hdc, &check_box_rc, check_bg, if check_hover { COLOR_ACCENT_HOVER } else { COLOR_CARD_BORDER }, 4);

    if state.auto_copy {
        SelectObject(hdc, state.hfont_small);
        SetTextColor(hdc, COLOR_TEXT_WHITE);
        let checkmark = to_wstring("✓");
        let mut cm_rc = check_box_rc;
        DrawTextW(hdc, checkmark.as_ptr(), (checkmark.len() - 1) as i32, &mut cm_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }

    SelectObject(hdc, state.hfont_regular);
    SetTextColor(hdc, if check_hover { COLOR_TEXT_WHITE } else { COLOR_TEXT_MUTED });
    let auto_copy_txt = to_wstring("Automatically copy HEX to clipboard when clicking screen area");
    TextOutW(hdc, 46, 350, auto_copy_txt.as_ptr(), (auto_copy_txt.len() - 1) as i32);

    // 7. Recent Colors Palette
    SelectObject(hdc, state.hfont_small);
    SetTextColor(hdc, COLOR_TEXT_MUTED);
    let hist_lbl = to_wstring("RECENT PALETTE (CLICK TO RESTORE & COPY)");
    TextOutW(hdc, 22, 396, hist_lbl.as_ptr(), (hist_lbl.len() - 1) as i32);

    for i in 0..10 {
        let sx = 22 + (i as i32 * 42);
        let swatch_rect = RECT { left: sx, top: 418, right: sx + 34, bottom: 452 };
        if i < state.recent_colors.len() {
            let c = state.recent_colors[i];
            let cr = rgb(c.0, c.1, c.2);
            let is_h = state.hovered_button == Some(ButtonId::RecentSwatch(i));
            draw_rounded_card(hdc, &swatch_rect, cr, if is_h { COLOR_TEXT_WHITE } else { COLOR_CARD_BORDER }, 6);
        } else {
            // Empty swatch placeholder
            draw_rounded_card(hdc, &swatch_rect, COLOR_CARD_BG, COLOR_CARD_BORDER, 6);
        }
    }

    // 8. Footer Status Bar
    let footer_rc = RECT { left: 0, top: 470, right: 460, bottom: 530 };
    let foot_brush = CreateSolidBrush(rgb(18, 16, 15));
    FillRect(hdc, &footer_rc, foot_brush);
    DeleteObject(foot_brush);

    SelectObject(hdc, state.hfont_small);
    SetTextColor(hdc, if state.copied_at.is_some() { COLOR_SUCCESS } else { COLOR_TEXT_MUTED });
    let status_w = to_wstring(&state.status_message);
    TextOutW(hdc, 20, 482, status_w.as_ptr(), (status_w.len() - 1) as i32);
}

// -------------------------------------------------------------
// Screen Eyedropper Magnifier Overlay Window
// -------------------------------------------------------------

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            SetCapture(hwnd);
            0
        }

        WM_MOUSEMOVE => {
            let mut pt: POINT = zeroed();
            GetCursorPos(&mut pt);

            if let Some(ol) = OVERLAY.as_mut() {
                ol.cursor_pt = pt;
                // Sample pixel from snapshot DC
                let colorref = GetPixel(ol.snapshot_dc, pt.x - ol.vx, pt.y - ol.vy);
                let r = (colorref & 0xFF) as u8;
                let g = ((colorref >> 8) & 0xFF) as u8;
                let b = ((colorref >> 16) & 0xFF) as u8;
                ol.current_col = (r, g, b);
            }

            InvalidateRect(hwnd, null(), 0);
            0
        }

        WM_LBUTTONUP => {
            // Screen area clicked! Pick this exact color!
            let mut pt: POINT = zeroed();
            GetCursorPos(&mut pt);

            let picked_col = if let Some(ol) = OVERLAY.as_ref() {
                let colorref = GetPixel(ol.snapshot_dc, pt.x - ol.vx, pt.y - ol.vy);
                let r = (colorref & 0xFF) as u8;
                let g = ((colorref >> 8) & 0xFF) as u8;
                let b = ((colorref >> 16) & 0xFF) as u8;
                (r, g, b)
            } else {
                (0, 0, 0)
            };

            cleanup_overlay(hwnd);

            // Restore main ColorPicker window
            if !MAIN_HWND.is_null() {
                ShowWindow(MAIN_HWND, SW_RESTORE);
                SetForegroundWindow(MAIN_HWND);

                if let Some(state) = STATE.as_mut() {
                    state.current_color = picked_col;
                    // Add to recent palette if not identical to the most recent
                    if state.recent_colors.front() != Some(&picked_col) {
                        state.recent_colors.push_front(picked_col);
                        if state.recent_colors.len() > 10 {
                            state.recent_colors.pop_back();
                        }
                    }

                    let hex = rgb_to_hex(picked_col.0, picked_col.1, picked_col.2);
                    if state.auto_copy {
                        copy_to_clipboard(MAIN_HWND, &hex);
                        state.copied_at = Some(Instant::now());
                        state.copied_text = "HEX".to_string();
                        state.status_message = format!("Picked & copied {} to clipboard!", hex);
                    } else {
                        state.status_message = format!("Picked {}. Click 'Copy HEX' to copy.", hex);
                    }

                    InvalidateRect(MAIN_HWND, null(), 0);
                }
            }
            0
        }

        WM_RBUTTONUP | WM_KEYDOWN => {
            // Cancel picking on right click or Esc key
            if msg == WM_RBUTTONUP || wparam == VK_ESCAPE as usize {
                cleanup_overlay(hwnd);

                if !MAIN_HWND.is_null() {
                    ShowWindow(MAIN_HWND, SW_RESTORE);
                    SetForegroundWindow(MAIN_HWND);
                    if let Some(state) = STATE.as_mut() {
                        state.status_message = "Color picking cancelled.".to_string();
                        InvalidateRect(MAIN_HWND, null(), 0);
                    }
                }
            }
            0
        }

        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            if let Some(ol) = OVERLAY.as_ref() {
                // Double-buffer overlay drawing
                let mem_dc = CreateCompatibleDC(hdc);
                let mem_bmp = CreateCompatibleBitmap(hdc, ol.vw, ol.vh);
                let old_bmp = SelectObject(mem_dc, mem_bmp);

                // Copy desktop snapshot
                BitBlt(mem_dc, 0, 0, ol.vw, ol.vh, ol.snapshot_dc, 0, 0, SRCCOPY);

                // Draw magnifier loupe on top
                draw_overlay_magnifier(mem_dc, ol);

                // Blit to screen
                BitBlt(hdc, 0, 0, ol.vw, ol.vh, mem_dc, 0, 0, SRCCOPY);

                SelectObject(mem_dc, old_bmp);
                DeleteObject(mem_bmp);
                DeleteDC(mem_dc);
            }

            EndPaint(hwnd, &ps);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// Draw the magnifying loupe and live color readout
unsafe fn draw_overlay_magnifier(hdc: HDC, ol: &OverlayData) {
    let pt = ol.cursor_pt;
    let col = ol.current_col;

    // Convert global cursor coordinates to overlay window coordinates
    let mx = pt.x - ol.vx;
    let my = pt.y - ol.vy;

    let grid_dim: i32 = 11; // 11x11 pixels
    let cell_size: i32 = 11;
    let grid_size: i32 = grid_dim * cell_size; // 121px

    // Position loupe offset from cursor
    let mut lx = mx + 28;
    let mut ly = my + 28;
    if lx + grid_size + 30 > ol.vw {
        lx = mx - grid_size - 28;
    }
    if ly + grid_size + 70 > ol.vh {
        ly = my - grid_size - 60;
    }

    let half_grid = grid_dim / 2;

    // Outer card container for loupe
    let card_rc = RECT {
        left: lx - 8,
        top: ly - 8,
        right: lx + grid_size + 8,
        bottom: ly + grid_size + 62,
    };
    draw_rounded_card(hdc, &card_rc, COLOR_CARD_BG, COLOR_CARD_BORDER, 12);

    // Draw magnified pixel grid
    for gy in 0..grid_dim {
        for gx in 0..grid_dim {
            let sx = mx - half_grid + gx;
            let sy = my - half_grid + gy;
            let p_col = GetPixel(ol.snapshot_dc, sx, sy);

            let px = lx + (gx * cell_size);
            let py = ly + (gy * cell_size);

            let p_brush = CreateSolidBrush(p_col);
            let cell_rc = RECT {
                left: px,
                top: py,
                right: px + cell_size,
                bottom: py + cell_size,
            };
            FillRect(hdc, &cell_rc, p_brush);
            DeleteObject(p_brush);

            // Pixel grid line
            let grid_pen = CreatePen(PS_SOLID as i32, 1, rgb(45, 45, 45));
            let old_pen = SelectObject(hdc, grid_pen);
            let old_br = SelectObject(hdc, GetStockObject(NULL_BRUSH as i32));
            Rectangle(hdc, px, py, px + cell_size + 1, py + cell_size + 1);
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_br);
            DeleteObject(grid_pen);
        }
    }

    // Draw center crosshair targeting the exact pixel
    let cx = lx + (half_grid * cell_size);
    let cy = ly + (half_grid * cell_size);
    let target_pen = CreatePen(PS_SOLID as i32, 2, rgb(255, 255, 255));
    let old_pen = SelectObject(hdc, target_pen);
    let old_br = SelectObject(hdc, GetStockObject(NULL_BRUSH as i32));
    Rectangle(hdc, cx, cy, cx + cell_size + 1, cy + cell_size + 1);
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_br);
    DeleteObject(target_pen);

    // Live color swatch preview badge
    let badge_y = ly + grid_size + 8;
    let badge_rc = RECT {
        left: lx,
        top: badge_y,
        right: lx + 26,
        bottom: badge_y + 26,
    };
    let swatch_ref = rgb(col.0, col.1, col.2);
    draw_rounded_card(hdc, &badge_rc, swatch_ref, COLOR_TEXT_WHITE, 6);

    // Large Monospace HEX String
    SetBkMode(hdc, TRANSPARENT as i32);
    let font_hex = create_font("Consolas", 18, true, true);
    let old_font = SelectObject(hdc, font_hex);
    SetTextColor(hdc, COLOR_TEXT_WHITE);
    let hex_w = to_wstring(&rgb_to_hex(col.0, col.1, col.2));
    TextOutW(hdc, lx + 34, badge_y + 3, hex_w.as_ptr(), (hex_w.len() - 1) as i32);

    // Subtitle helper
    let font_sub = create_font("Segoe UI", 11, false, false);
    SelectObject(hdc, font_sub);
    SetTextColor(hdc, COLOR_TEXT_MUTED);
    let helper_w = to_wstring("Click: Pick & Copy | Esc: Cancel");
    TextOutW(hdc, lx, badge_y + 30, helper_w.as_ptr(), (helper_w.len() - 1) as i32);

    SelectObject(hdc, old_font);
    DeleteObject(font_hex);
    DeleteObject(font_sub);
}

// Clean up overlay resources
unsafe fn cleanup_overlay(hwnd: HWND) {
    ReleaseCapture();
    DestroyWindow(hwnd);
    OVERLAY_HWND = null_mut();

    if let Some(ol) = OVERLAY.take() {
        SelectObject(ol.snapshot_dc, ol.old_bmp);
        DeleteObject(ol.snapshot_bmp);
        DeleteDC(ol.snapshot_dc);
    }
}

// Launch the full-screen transparent magnifier overlay
unsafe fn start_screen_picker() {
    let hinstance = GetModuleHandleW(null());

    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

    // Capture screen snapshot
    let desktop_dc = GetDC(null_mut());
    let snapshot_dc = CreateCompatibleDC(desktop_dc);
    let snapshot_bmp = CreateCompatibleBitmap(desktop_dc, vw, vh);
    let old_bmp = SelectObject(snapshot_dc, snapshot_bmp);

    BitBlt(snapshot_dc, 0, 0, vw, vh, desktop_dc, vx, vy, SRCCOPY);
    ReleaseDC(null_mut(), desktop_dc);

    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);

    let colorref = GetPixel(snapshot_dc, pt.x - vx, pt.y - vy);
    let r = (colorref & 0xFF) as u8;
    let g = ((colorref >> 8) & 0xFF) as u8;
    let b = ((colorref >> 16) & 0xFF) as u8;

    OVERLAY = Some(OverlayData {
        vx,
        vy,
        vw,
        vh,
        snapshot_dc,
        snapshot_bmp,
        old_bmp,
        cursor_pt: pt,
        current_col: (r, g, b),
    });

    let class_name = to_wstring("ChromaOverlayClass");
    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(overlay_wnd_proc);
    wc.hInstance = hinstance;
    wc.lpszClassName = class_name.as_ptr();
    wc.hCursor = LoadCursorW(null_mut(), IDC_CROSS);
    RegisterClassW(&wc);

    let overlay = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        class_name.as_ptr(),
        to_wstring("ChromaOverlay").as_ptr(),
        WS_POPUP | WS_VISIBLE,
        vx,
        vy,
        vw,
        vh,
        null_mut(),
        null_mut(),
        hinstance,
        null(),
    );

    OVERLAY_HWND = overlay;

    ShowWindow(overlay, SW_SHOW);
    UpdateWindow(overlay);
    SetForegroundWindow(overlay);
}

// -------------------------------------------------------------
// Entry Point
// -------------------------------------------------------------

fn main() {
    unsafe {
        // Enable per-monitor DPI awareness if supported
        SetProcessDPIAware();

        let hinstance = GetModuleHandleW(null());

        // Load embedded application icon (ID 1 from resource.rc)
        let mut icon = LoadIconW(hinstance, 1 as *const u16);
        if icon.is_null() {
            icon = LoadIconW(null_mut(), IDI_APPLICATION);
        }

        // Initialize state
        let mut recent = VecDeque::new();
        recent.push_back((79, 70, 229));   // Indigo #4F46E5
        recent.push_back((16, 185, 129));  // Emerald #10B981
        recent.push_back((245, 158, 11));  // Amber #F59E0B
        recent.push_back((239, 68, 68));   // Red #EF4444
        recent.push_back((59, 130, 246));  // Blue #3B82F6
        recent.push_back((168, 85, 247));  // Purple #A855F7
        recent.push_back((236, 72, 153));  // Pink #EC4899
        recent.push_back((24, 24, 27));    // Dark #18181B

        STATE = Some(AppState {
            current_color: (79, 70, 229), // default #4F46E5
            recent_colors: recent,
            copied_at: None,
            copied_text: String::new(),
            auto_copy: true,
            hovered_button: None,
            pressed_button: None,
            status_message: "Ready. Click 'Pick Color' or press Space to sample any pixel.".to_string(),
            hfont_title: create_font("Segoe UI", 20, true, false),
            hfont_hex: create_font("Consolas", 24, true, true),
            hfont_medium: create_font("Segoe UI", 15, true, false),
            hfont_regular: create_font("Segoe UI", 14, false, false),
            hfont_small: create_font("Segoe UI", 12, false, false),
            app_icon: icon,
        });

        // Register window class
        let class_name = to_wstring("ChromaPickWindowClass");
        let mut wc: WNDCLASSW = zeroed();
        wc.lpfnWndProc = Some(main_wnd_proc);
        wc.hInstance = hinstance;
        wc.lpszClassName = class_name.as_ptr();
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        wc.hIcon = icon;
        RegisterClassW(&wc);

        // Center on screen
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let wx = (screen_w - WINDOW_WIDTH) / 2;
        let wy = (screen_h - WINDOW_HEIGHT) / 2;

        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class_name.as_ptr(),
            to_wstring("ChromaPick - Screen Color Picker").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE,
            wx,
            wy,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        );

        MAIN_HWND = hwnd;

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        // Standard message loop
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
