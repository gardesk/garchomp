//! X11 atom definitions for EWMH and compositor protocols.

use x11rb::atom_manager;

atom_manager! {
    /// Atoms used by garchomp.
    pub Atoms: AtomsCookie {
        // ICCCM
        WM_PROTOCOLS,
        WM_DELETE_WINDOW,
        WM_STATE,
        WM_TRANSIENT_FOR,

        // Text types
        UTF8_STRING,

        // EWMH - Window types
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DESKTOP,
        _NET_WM_WINDOW_TYPE_DOCK,
        _NET_WM_WINDOW_TYPE_TOOLBAR,
        _NET_WM_WINDOW_TYPE_MENU,
        _NET_WM_WINDOW_TYPE_UTILITY,
        _NET_WM_WINDOW_TYPE_SPLASH,
        _NET_WM_WINDOW_TYPE_DIALOG,
        _NET_WM_WINDOW_TYPE_DROPDOWN_MENU,
        _NET_WM_WINDOW_TYPE_POPUP_MENU,
        _NET_WM_WINDOW_TYPE_TOOLTIP,
        _NET_WM_WINDOW_TYPE_NOTIFICATION,
        _NET_WM_WINDOW_TYPE_COMBO,
        _NET_WM_WINDOW_TYPE_DND,
        _NET_WM_WINDOW_TYPE_NORMAL,

        // EWMH - Window state
        _NET_WM_STATE,
        _NET_WM_STATE_MODAL,
        _NET_WM_STATE_STICKY,
        _NET_WM_STATE_MAXIMIZED_VERT,
        _NET_WM_STATE_MAXIMIZED_HORZ,
        _NET_WM_STATE_SHADED,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_STATE_HIDDEN,
        _NET_WM_STATE_FULLSCREEN,
        _NET_WM_STATE_ABOVE,
        _NET_WM_STATE_BELOW,
        _NET_WM_STATE_DEMANDS_ATTENTION,
        _NET_WM_STATE_FOCUSED,

        // EWMH - Window name
        _NET_WM_NAME,

        // EWMH - Active window
        _NET_ACTIVE_WINDOW,
        _NET_CLIENT_LIST,
        _NET_CLIENT_LIST_STACKING,

        // EWMH - Desktop/Workspace
        _NET_WM_DESKTOP,
        _NET_CURRENT_DESKTOP,
        _NET_NUMBER_OF_DESKTOPS,

        // Compositor bypass
        _NET_WM_BYPASS_COMPOSITOR,

        // Opacity
        _NET_WM_WINDOW_OPACITY,

        // Root pixmap (wallpaper)
        _XROOTPMAP_ID,
        ESETROOT_PMAP_ID,

        // Compositor-specific
        _GARCHOMP_COLORSPACE,
        _GARCHOMP_MAX_LUMINANCE,
        _GARCHOMP_MIN_LUMINANCE,
    }
}
