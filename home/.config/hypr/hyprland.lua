---- MONITORS ----
------------------

-- See https://wiki.hypr.land/configuring/core/monitors/
hl.monitor({
    output   = "",
    mode     = "preferred",
    position = "auto",
    scale    = "auto",
})


---------------------
---- MY PROGRAMS ----
---------------------

-- Set programs that you use
local home        = os.getenv("HOME") or ""
local terminal    = "kitty"
local fileManager = "thunar"
local menu        = "~/.local/bin/asura-launcher"


-------------------
---- AUTOSTART ----
-------------------

-- See https://wiki.hypr.land/configuring/core/autostart/

-- Autostart necessary processes (like notifications daemons, status bars, etc.)
-- Or execute your favorite apps at launch like this:

 hl.on("hyprland.start", function () 
   
   hl.exec_cmd("waybar")
   hl.exec_cmd("mako")
   hl.exec_cmd("awww-daemon")
   hl.exec_cmd("wl-paste --type text --watch cliphist store")
   hl.exec_cmd("wl-paste --type image --watch cliphist store")
   hl.exec_cmd("/usr/lib/polkit-gnome/polkit-gnome-authentication-agent-1")
   hl.exec_cmd(home .. "/.config/hypr/sync-lock-theme.sh")
 end)


-------------------------------
---- ENVIRONMENT VARIABLES ----
-------------------------------

-- See https://wiki.hypr.land/configuring/core/environment-variables/

hl.env("XCURSOR_SIZE", "24")
hl.env("HYPRCURSOR_SIZE", "24")
hl.env("GSK_RENDERER", "gl")
hl.env("GDK_DISABLE", "vulkan")
pcall(require, "host")


-----------------------
----- PERMISSIONS -----
-----------------------

-- See https://wiki.hypr.land/configuring/core/advanced-configuration/permissions/
-- Please note permission changes here require a Hyprland restart and are not applied on-the-fly
-- for security reasons

-- hl.config({
--   ecosystem = {
--     enforce_permissions = true,
--   },
-- })

-- hl.permission("/usr/(bin|local/bin)/grim", "screencopy", "allow")
-- hl.permission("/usr/(lib|libexec|lib64)/xdg-desktop-portal-hyprland", "screencopy", "allow")
-- hl.permission("/usr/(bin|local/bin)/hyprpm", "plugin", "allow")


-----------------------
---- LOOK AND FEEL ----
-----------------------

-- Refer to https://wiki.hypr.land/configuring/core/config-options/
hl.config({
    general = {
        gaps_in  = 5,
        gaps_out = 20,

        border_size = 2,

        col = {
            active_border   = { colors = {"rgba(33ccffee)", "rgba(00ff99ee)"}, angle = 45 },
            inactive_border = "rgba(595959aa)",
        },

        -- Set to true to enable resizing windows by clicking and dragging on borders and gaps
        resize_on_border = false,

        -- Please see https://wiki.hypr.land/configuring/extra/tearing/ before you turn this on
        allow_tearing = false,

        layout = "scrolling",
    },

    decoration = {
        rounding       = 12,
        rounding_power = 2,

        -- Opaque windows avoid expensive multi-pass fullscreen blur on weaker GPUs;
        -- translucent surfaces (kitty, waybar, launcher) use explicit rules below.
        active_opacity   = 1.0,
        inactive_opacity = 1.0,

        shadow = {
            enabled      = true,
            range        = 10,
            render_power = 3,
            color        = 0x88000000,
        },

        blur = {
            enabled           = true,
            size              = 5,
            passes            = 2,
            new_optimizations = true,
            vibrancy          = 0.22,
            vibrancy_darkness = 0.0,
        },
    },

    animations = {
        enabled = true,
    },
})

-- Smooth fluid curves & animations (inspired by Asura)
hl.curve("specialWorkSwitch", { type = "bezier", points = { { 0.12, 0.85 }, { 0.22, 1 } } })
hl.curve("emphasizedDecel",   { type = "bezier", points = { { 0.14, 0.90 }, { 0.22, 1 } } })
hl.curve("softOut",           { type = "bezier", points = { { 0.22, 1 },    { 0.36, 1 } } })
hl.curve("fluid",             { type = "bezier", points = { { 0.16, 1 },    { 0.30, 1 } } })
hl.curve("smoothFade",        { type = "bezier", points = { { 0.33, 1 },    { 0.68, 1 } } })

hl.animation({ leaf = "global",           enabled = true, speed = 6.0, bezier = "fluid" })
hl.animation({ leaf = "border",           enabled = true, speed = 7.0, bezier = "smoothFade" })
hl.animation({ leaf = "windows",          enabled = true, speed = 5.5, bezier = "fluid" })
hl.animation({ leaf = "windowsIn",        enabled = true, speed = 5.0, bezier = "fluid",             style = "popin 84%" })
hl.animation({ leaf = "windowsOut",       enabled = true, speed = 4.5, bezier = "softOut",           style = "popin 86%" })
hl.animation({ leaf = "windowsMove",      enabled = true, speed = 6.5, bezier = "fluid" })
hl.animation({ leaf = "fade",             enabled = true, speed = 5.0, bezier = "smoothFade" })
hl.animation({ leaf = "fadeIn",           enabled = true, speed = 4.5, bezier = "smoothFade" })
hl.animation({ leaf = "fadeOut",          enabled = true, speed = 4.0, bezier = "smoothFade" })
hl.animation({ leaf = "fadeDim",          enabled = true, speed = 5.5, bezier = "smoothFade" })
hl.animation({ leaf = "layers",           enabled = true, speed = 4.5, bezier = "emphasizedDecel" })
hl.animation({ leaf = "layersIn",         enabled = true, speed = 5.0, bezier = "fluid",             style = "popin 86%" })
hl.animation({ leaf = "layersOut",        enabled = true, speed = 4.2, bezier = "softOut",           style = "popin 90%" })
hl.animation({ leaf = "fadeLayers",       enabled = true, speed = 4.5, bezier = "smoothFade" })
hl.animation({ leaf = "fadeLayersIn",     enabled = true, speed = 5.0, bezier = "smoothFade" })
hl.animation({ leaf = "fadeLayersOut",    enabled = true, speed = 4.0, bezier = "smoothFade" })
hl.animation({ leaf = "workspaces",       enabled = true, speed = 6.0, bezier = "fluid",             style = "slidevert" })
hl.animation({ leaf = "workspacesIn",     enabled = true, speed = 6.0, bezier = "fluid",             style = "slidevert" })
hl.animation({ leaf = "workspacesOut",    enabled = true, speed = 6.0, bezier = "fluid",             style = "slidevert" })
hl.animation({ leaf = "specialWorkspace", enabled = true, speed = 5.5, bezier = "specialWorkSwitch", style = "slidefadevert 15%" })
hl.animation({ leaf = "zoomFactor",       enabled = true, speed = 6.0, bezier = "fluid" })

-- Ref https://wiki.hypr.land/configuring/core/rules/workspace-rules/
-- "Smart gaps" / "No gaps when only"
-- uncomment all if you wish to use that.
-- hl.workspace_rule({ workspace = "w[tv1]", gaps_out = 0, gaps_in = 0 })
-- hl.workspace_rule({ workspace = "f[1]",   gaps_out = 0, gaps_in = 0 })
-- hl.window_rule({
--     name  = "no-gaps-wtv1",
--     match = { float = false, workspace = "w[tv1]" },
--     border_size = 0,
--     rounding    = 0,
-- })
-- hl.window_rule({
--     name  = "no-gaps-f1",
--     match = { float = false, workspace = "f[1]" },
--     border_size = 0,
--     rounding    = 0,
-- })

-- See https://wiki.hypr.land/configuring/layouts/dwindle-layout/ for more
hl.config({
    dwindle = {
        preserve_split = true, -- You probably want this
    },
})

-- See https://wiki.hypr.land/configuring/layouts/master-layout/ for more
hl.config({
    master = {
        new_status = "master",
    },
})

-- See https://wiki.hypr.land/configuring/layouts/scrolling-layout/ for more
hl.config({
    scrolling = {
        fullscreen_on_one_column = true,
        column_width             = 0.5,
        focus_fit_method         = 1, -- 1 = fit, 0 = center
        follow_focus             = 1,
        direction                = "right",
        wrap_focus               = false,
    },
})

----------------
----  MISC  ----
----------------

hl.config({
    misc = {
        force_default_wallpaper  = 0,    -- Set to 0 or 1 to disable the anime mascot wallpapers
        disable_hyprland_logo    = true, -- If true disables the random hyprland logo / anime girl background. :(
        disable_splash_rendering = true,
        render_unfocused_fps     = 10,
    },
    render = {
        direct_scanout = 2,
        cm_enabled     = false,
    },
    xwayland = {
        enabled = true,
    },
})


----------------
---- LAYERS ----
----------------

hl.config({
    layers = {
        allow_new_top_layers_over_existing_fullscreen = true, -- Set to false to make newly mapped top layers render below existing covering fullscreen window
    },
})


---------------
---- INPUT ----
---------------

hl.config({
    input = {
        kb_layout  = "us, ru",
        kb_variant = "",
        kb_model   = "",
        kb_options = "grp:alt_shift_toggle",
        kb_rules   = "",

        follow_mouse = 1,

        sensitivity = 0, -- -1.0 - 1.0, 0 means no modification.

        touchpad = {
            natural_scroll = false,
        },
    },
})

hl.gesture({
    fingers = 3,
    direction = "horizontal",
    action = "workspace"
})


---------------------
---- KEYBINDINGS ----
---------------------

local mainMod = "SUPER" -- Sets "Windows" key as main modifier

-- Example binds, see https://wiki.hypr.land/configuring/core/binds/ for more
-- Niri-style dynamic workspaces: only occupied workspaces + 1 trailing empty workspace can exist
-- Per-workspace horizontal peek state (strictly within the same workspace):
--   { mode = "clean", side = "left" | "right", delta = px, win_addr = addr }
--   { mode = "half_right", delta = px, win_addr = addr }
local peek_state = {}
local internal_nav = false

local function win_is_fullscreen(w)
    return w ~= nil and w.fullscreen ~= nil and w.fullscreen ~= 0
end

local function vec_x(v)
    if type(v) ~= "table" then return nil end
    return v.x or v[1]
end

local function win_right_edge(w)
    if not w then return nil end
    local x = vec_x(w.at)
    local width = vec_x(w.size)
    if x and width then
        return x + width
    end
    return nil
end

local function active_monitor_width()
    local mon = hl.get_active_monitor()
    if mon then
        local w = mon.width or vec_x(mon.size) or 1920
        local s = mon.scale or 1
        if s > 0 then
            return math.floor(w / s)
        end
        return math.floor(w)
    end
    return 1920
end

local function count_ws_apps(ws_id)
    local n = 0
    for _, w in ipairs(hl.get_workspace_windows(ws_id)) do
        if w.mapped and not w.hidden then
            n = n + 1
        end
    end
    return n
end

local function find_ws_window_by_addr(ws_id, addr)
    if not addr then return nil end
    for _, w in ipairs(hl.get_workspace_windows(ws_id)) do
        if w.address == addr then
            return w
        end
    end
    return nil
end

local function run_internal(fn)
    internal_nav = true
    local ok, err = pcall(fn)
    internal_nav = false
    if not ok then
        error(err)
    end
end

hl.bind(mainMod .. " + Return", hl.dsp.exec_cmd(terminal))
local closeWindowBind = hl.bind(mainMod .. " + Q", hl.dsp.window.close())
-- closeWindowBind:set_enabled(false)
hl.bind(mainMod .. " + M", hl.dsp.exit())
hl.bind(mainMod .. " + L", hl.dsp.exec_cmd(home .. "/.config/hypr/lock.sh"))
hl.bind(mainMod .. " + E", hl.dsp.exec_cmd(fileManager))
hl.bind(mainMod .. " + V", hl.dsp.window.float({ action = "toggle" }))
hl.bind(mainMod .. " + F", function()
    local win = hl.get_active_window()
    if not win then return end
    local ws = hl.get_active_workspace()
    if ws and ws.id then
        peek_state[ws.id] = nil
    end
    hl.dispatch(hl.dsp.window.fullscreen({ action = "toggle", mode = "fullscreen", layout_aware = true }))
end)
hl.bind(mainMod .. " + SPACE", hl.dsp.exec_cmd(menu))
hl.bind(mainMod .. " + P", hl.dsp.window.pseudo())
hl.bind(mainMod .. " + J", hl.dsp.layout("togglesplit"))    -- dwindle only
local function occupied_workspaces()
    local list = {}
    for _, ws in ipairs(hl.get_workspaces()) do
        if not ws.special and ws.id and ws.id >= 1 and not ws.is_empty then
            table.insert(list, ws)
        end
    end
    table.sort(list, function(a, b) return a.id < b.id end)
    return list
end

local function max_occupied_id()
    local max_id = 0
    for _, ws in ipairs(occupied_workspaces()) do
        if ws.id > max_id then
            max_id = ws.id
        end
    end
    return max_id
end

local function active_workspace_id()
    local cur = hl.get_active_workspace()
    if cur and not cur.special and cur.id and cur.id >= 1 then
        return cur.id
    end
    return 1
end

local compacting = false
local function compact_workspaces()
    if compacting then return end
    compacting = true

    for idx, ws in ipairs(occupied_workspaces()) do
        if ws.id ~= idx then
            peek_state = {}
            local blocker = hl.get_workspace(idx)
            if blocker and blocker.is_empty then
                hl.dispatch(hl.dsp.focus({ workspace = ws.id }))
            end
            hl.dispatch(hl.dsp.workspace.change_id({ workspace = ws.id, id = idx }))
        end
    end

    local cur = hl.get_active_workspace()
    local max_ws = max_occupied_id() + 1
    if cur and not cur.special and cur.id and cur.id > max_ws and cur.is_empty then
        hl.dispatch(hl.dsp.focus({ workspace = max_ws }))
    end

    compacting = false
end

local function schedule_compact()
    hl.timer(compact_workspaces, { timeout = 20, type = "oneshot" })
end

hl.on("window.open", function()
    peek_state = {}
end)
hl.on("window.destroy", function()
    peek_state = {}
    schedule_compact()
end)
hl.on("window.move_to_workspace", function()
    peek_state = {}
    schedule_compact()
end)
hl.on("window.fullscreen", function()
    if internal_nav then return end
    local ws = hl.get_active_workspace()
    if ws and ws.id then
        peek_state[ws.id] = nil
    end
end)
hl.on("window.active", function()
    if internal_nav then return end
    local active = hl.get_active_window()
    if active ~= nil then
        local ws = hl.get_active_workspace()
        if ws and ws.id then
            local st = peek_state[ws.id]
            if st and active.address ~= st.win_addr then
                peek_state[ws.id] = nil
            end
        end
    end
end)

local function switch_workspace(target)
    if target >= 1 and target <= max_occupied_id() + 1 then
        hl.dispatch(hl.dsp.focus({ workspace = target }))
    end
end

local function step_workspace(delta)
    switch_workspace(active_workspace_id() + delta)
end

local function move_window_workspace(target)
    if target >= 1 and target <= max_occupied_id() + 1 then
        hl.dispatch(hl.dsp.window.move({ workspace = target }))
        schedule_compact()
    end
end

local function cycle_workspace(delta)
    local max_ws = math.max(1, max_occupied_id())
    local cur = active_workspace_id()
    if cur > max_ws then max_ws = cur end
    local target = ((cur - 1 + delta) % max_ws) + 1
    hl.dispatch(hl.dsp.focus({ workspace = target }))
end

local function navigate_horizontal(dir)
    local cur_ws = hl.get_active_workspace()
    if not cur_ws or not cur_ws.id then
        return
    end

    local ws_id = cur_ws.id
    local st = peek_state[ws_id]

    -- 1. If currently showing a completely clean workspace (only 1 fullscreen app slid off-screen)
    if st and st.mode == "clean" then
        if (st.side == "right" and dir == "left") or (st.side == "left" and dir == "right") then
            peek_state[ws_id] = nil
            local move_arg = (st.side == "right") and ("move +" .. st.delta) or ("move -" .. st.delta)
            run_internal(function()
                hl.dispatch(hl.dsp.layout(move_arg))
                local target = find_ws_window_by_addr(ws_id, st.win_addr)
                local active = hl.get_active_window()
                if target and (active == nil or active.address ~= st.win_addr) then
                    hl.dispatch(hl.dsp.focus({ window = target }))
                end
            end)
        end
        return
    end

    -- 2. If currently peeking the unused right half of the workspace
    if st and st.mode == "half_right" then
        if dir == "left" then
            peek_state[ws_id] = nil
            run_internal(function()
                hl.dispatch(hl.dsp.layout("move +" .. st.delta))
                local target = find_ws_window_by_addr(ws_id, st.win_addr)
                if target then
                    hl.dispatch(hl.dsp.focus({ window = target }))
                end
            end)
        end
        return
    end

    -- 3. Normal navigation strictly on the current workspace
    if cur_ws.is_empty then
        return
    end

    local cur_win = hl.get_active_window()
    if not cur_win then
        return
    end

    if cur_win.floating then
        hl.dispatch(hl.dsp.focus({ direction = dir }))
        return
    end

    -- 3a. Single fullscreen app with NO other apps on this workspace -> reveal 100% clean desktop on SAME workspace (left or right)
    if win_is_fullscreen(cur_win) and count_ws_apps(ws_id) <= 1 then
        local slide_w = active_monitor_width() + 40
        local move_arg = (dir == "right") and ("move -" .. slide_w) or ("move +" .. slide_w)
        peek_state[ws_id] = {
            mode = "clean",
            side = dir,
            delta = slide_w,
            win_addr = cur_win.address,
        }
        run_internal(function()
            hl.dispatch(hl.dsp.layout(move_arg))
        end)
        return
    end

    -- 3b. Scroll/focus neighbor column on the SAME workspace (preserves fullscreen state via CScrollingFullscreenHandler)
    local before_addr = cur_win.address
    local short_dir = (dir == "right") and "r" or "l"
    run_internal(function()
        hl.dispatch(hl.dsp.layout("focus " .. short_dir))
    end)
    local after_win = hl.get_active_window()

    if after_win ~= nil and after_win.address ~= before_addr then
        return
    end

    -- 3c. At the rightmost column -> shift left by half usable width to open unused right half of SAME workspace
    if dir == "right" then
        local mon_w = active_monitor_width()
        local right_edge = win_right_edge(cur_win)
        if right_edge == nil or right_edge > mon_w * 0.65 then
            local half_w = math.floor((mon_w - 40) * 0.5)
            peek_state[ws_id] = {
                mode = "half_right",
                delta = half_w,
                win_addr = before_addr,
            }
            run_internal(function()
                hl.dispatch(hl.dsp.layout("move -" .. half_w))
            end)
        end
    end
end

-- Switch focus left/right between columns (niri-style) and switch workspaces with up/down
hl.bind(mainMod .. " + left",  function() navigate_horizontal("left") end)
hl.bind(mainMod .. " + right", function() navigate_horizontal("right") end)
hl.bind(mainMod .. " + up",    function() step_workspace(-1) end)
hl.bind(mainMod .. " + down",  function() step_workspace(1) end)

-- Switch workspaces with mainMod + [0-9]
-- Move active window to a workspace with mainMod + SHIFT + [0-9]
for i = 1, 10 do
    local key = i % 10 -- 10 maps to key 0
    hl.bind(mainMod .. " + " .. key,             function() switch_workspace(i) end)
    hl.bind(mainMod .. " + SHIFT + " .. key,     function() move_window_workspace(i) end)
end

-- Example special workspace (scratchpad)
hl.bind(mainMod .. " + S",         hl.dsp.workspace.toggle_special("magic"))
hl.bind(mainMod .. " + SHIFT + S", hl.dsp.window.move({ workspace = "special:magic" }))

-- Window & workspace cycling
hl.bind("ALT + Tab",           hl.dsp.window.cycle_next({ next = true }))
hl.bind("ALT + SHIFT + Tab",   hl.dsp.window.cycle_next({ next = false }))
hl.bind(mainMod .. " + Tab",         function() cycle_workspace(1) end)
hl.bind(mainMod .. " + SHIFT + Tab", function() cycle_workspace(-1) end)

-- Wallpaper picker (coverflow): Win + Shift (order- & layout-independent, ignores Win+Shift+<key> chords)
local ws_super_down = false
local ws_shift_down = false
local ws_chord_armed = false
local ws_chord_dirty = false
local picker_debouncing = false

local function trigger_wallfx_picker()
    if picker_debouncing then return end
    picker_debouncing = true
    hl.timer(function() picker_debouncing = false end, { timeout = 300, type = "oneshot" })
    hl.dispatch(hl.dsp.exec_cmd("~/.local/bin/wallfx picker"))
end

hl.on("input.keyboard.key", function(keycode, _, state)
    local is_super = (keycode == 133 or keycode == 134) -- Super_L / Super_R
    local is_shift = (keycode == 50  or keycode == 62)  -- Shift_L / Shift_R
    local pressed  = (state == 1)

    if is_super then
        if pressed then
            ws_super_down = true
            if ws_shift_down and not ws_chord_dirty then
                ws_chord_armed = true
            end
        else
            local fire = ws_chord_armed and not ws_chord_dirty
            ws_super_down = false
            ws_chord_armed = false
            if not ws_shift_down then
                ws_chord_dirty = false
            end
            if fire then
                trigger_wallfx_picker()
            end
        end
    elseif is_shift then
        if pressed then
            ws_shift_down = true
            if ws_super_down and not ws_chord_dirty then
                ws_chord_armed = true
            end
        else
            local fire = ws_chord_armed and not ws_chord_dirty
            ws_shift_down = false
            ws_chord_armed = false
            if not ws_super_down then
                ws_chord_dirty = false
            end
            if fire then
                trigger_wallfx_picker()
            end
        end
    else
        if pressed and (ws_super_down or ws_shift_down) then
            ws_chord_armed = false
            ws_chord_dirty = true
        end
    end
end)

-- Scroll through existing workspaces with mainMod + scroll
hl.bind(mainMod .. " + mouse_down", function() step_workspace(1) end)
hl.bind(mainMod .. " + mouse_up",   function() step_workspace(-1) end)

-- Move/resize windows with mainMod + LMB/RMB and dragging
hl.bind(mainMod .. " + mouse:272", hl.dsp.window.drag(),   { mouse = true })
hl.bind(mainMod .. " + mouse:273", hl.dsp.window.resize(), { mouse = true })

-- Laptop multimedia keys for volume and LCD brightness
hl.bind("XF86AudioRaiseVolume", hl.dsp.exec_cmd("wpctl set-volume -l 1 @DEFAULT_AUDIO_SINK@ 5%+"), { locked = true, repeating = true })
hl.bind("XF86AudioLowerVolume", hl.dsp.exec_cmd("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"),      { locked = true, repeating = true })
hl.bind("XF86AudioMute",        hl.dsp.exec_cmd("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"),     { locked = true, repeating = true })
hl.bind("XF86AudioMicMute",     hl.dsp.exec_cmd("wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle"),   { locked = true, repeating = true })
hl.bind("XF86MonBrightnessUp",  hl.dsp.exec_cmd("~/.local/bin/smooth-brightness up 5"),   { locked = true, repeating = true })
hl.bind("XF86MonBrightnessDown",hl.dsp.exec_cmd("~/.local/bin/smooth-brightness down 5"), { locked = true, repeating = true })

-- Requires playerctl
hl.bind("XF86AudioNext",  hl.dsp.exec_cmd("playerctl next"),       { locked = true })
hl.bind("XF86AudioPause", hl.dsp.exec_cmd("playerctl play-pause"), { locked = true })
hl.bind("XF86AudioPlay",  hl.dsp.exec_cmd("playerctl play-pause"), { locked = true })
hl.bind("XF86AudioPrev",  hl.dsp.exec_cmd("playerctl previous"),   { locked = true })


--------------------------------
---- WINDOWS AND WORKSPACES ----
--------------------------------

-- See https://wiki.hypr.land/configuring/core/rules/

-- Example window rules that are useful

local suppressMaximizeRule = hl.window_rule({
    -- Ignore maximize requests from all apps. You'll probably like this.
    name  = "suppress-maximize-events",
    match = { class = ".*" },

    suppress_event = "maximize",
})
-- suppressMaximizeRule:set_enabled(false)

hl.window_rule({
    -- Fix some dragging issues with XWayland
    name  = "fix-xwayland-drags",
    match = {
        class      = "^$",
        title      = "^$",
        xwayland   = true,
        float      = true,
        fullscreen = false,
        pin        = false,
    },

    no_focus = true,
})

-- Layer rules also return a handle.
-- local overlayLayerRule = hl.layer_rule({
--     name  = "no-anim-overlay",
--     match = { namespace = "^my-overlay$" },
--     no_anim = true,
-- })
-- overlayLayerRule:set_enabled(false)

-- Hyprland-run windowrule
hl.window_rule({
    name  = "move-hyprland-run",
    match = { class = "hyprland-run" },

    move  = "20 monitor_h-120",
    float = true,
})

hl.window_rule({
    name    = "kitty-glass",
    match   = { class = "^kitty$" },
    opacity = "1.0 override 0.94 override",
})

hl.layer_rule({
    name         = "waybar-rounded",
    match        = { namespace = "^waybar$" },
    blur         = true,
    ignore_alpha = 0.2,
})

hl.layer_rule({
    name         = "rofi-asura",
    match        = { namespace = "^rofi$" },
    blur         = true,
    ignore_alpha = 0.2,
    animation    = "popin 82%",
})

hl.layer_rule({
    name         = "asura-launcher",
    match        = { namespace = "^asura-launcher$" },
    blur         = true,
    ignore_alpha = 0.2,
    animation    = "popin 82%",
})

-- Dynamic wallpaper theme generated by wallfx
pcall(require, "theme")

hl.on("config.reloaded", function()
    hl.exec_cmd(home .. "/.config/hypr/sync-lock-theme.sh")
end)
