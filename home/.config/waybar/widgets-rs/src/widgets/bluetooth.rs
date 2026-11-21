use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use glib::ControlFlow;
use gtk::pango::EllipsizeMode;
use gtk::prelude::*;

use crate::common::{clear_container, make_section_label, run_bg, Align, WidgetPopup};

const BLUEZ_BUS: &str = "org.bluez";
const AGENT_PATH: &str = "/org/waybar/bt_agent";
const AGENT_XML: &str = r#"
<node>
  <interface name="org.bluez.Agent1">
    <method name="Release"/>
    <method name="RequestPinCode">
      <arg type="o" name="device" direction="in"/>
      <arg type="s" name="pincode" direction="out"/>
    </method>
    <method name="DisplayPinCode">
      <arg type="o" name="device" direction="in"/>
      <arg type="s" name="pincode" direction="in"/>
    </method>
    <method name="RequestPasskey">
      <arg type="o" name="device" direction="in"/>
      <arg type="u" name="passkey" direction="out"/>
    </method>
    <method name="DisplayPasskey">
      <arg type="o" name="device" direction="in"/>
      <arg type="u" name="passkey" direction="in"/>
      <arg type="q" name="entered" direction="in"/>
    </method>
    <method name="RequestConfirmation">
      <arg type="o" name="device" direction="in"/>
      <arg type="u" name="passkey" direction="in"/>
    </method>
    <method name="RequestAuthorization">
      <arg type="o" name="device" direction="in"/>
    </method>
    <method name="AuthorizeService">
      <arg type="o" name="device" direction="in"/>
      <arg type="s" name="uuid" direction="in"/>
    </method>
    <method name="Cancel"/>
  </interface>
</node>
"#;

#[derive(Clone)]
struct BtDevice {
    path: String,
    name: String,
    connected: bool,
    paired: bool,
    icon: &'static str,
    battery: Option<String>,
}

struct BluezSnapshot {
    adapter: Option<String>,
    powered: bool,
    discovering: bool,
    devices: Vec<BtDevice>,
}

fn device_icon(icon_name: Option<&str>) -> &'static str {
    match icon_name.unwrap_or("") {
        "audio-headset" => "󰋎",
        "audio-headphones" => "󰋋",
        "audio-card" => "󰓃",
        "input-mouse" => "󰍽",
        "input-keyboard" => "󰌌",
        "input-gaming" => "󰊴",
        "phone" => "󰏲",
        "computer" => "󰌢",
        _ => "󰂯",
    }
}

fn sys_bus() -> Result<gio::DBusConnection, glib::Error> {
    gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
}

fn get_bluez_state() -> BluezSnapshot {
    let Ok(bus) = sys_bus() else {
        return BluezSnapshot {
            adapter: None,
            powered: false,
            discovering: false,
            devices: Vec::new(),
        };
    };

    let Ok(res) = bus.call_sync(
        Some(BLUEZ_BUS),
        "/",
        "org.freedesktop.DBus.ObjectManager",
        "GetManagedObjects",
        None,
        Some(glib::VariantTy::new("(a{oa{sa{sv}}})").unwrap()),
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    ) else {
        return BluezSnapshot {
            adapter: None,
            powered: false,
            discovering: false,
            devices: Vec::new(),
        };
    };

    let objects = res.child_value(0);
    let mut adapter_path: Option<String> = None;
    let mut powered = false;
    let mut discovering = false;
    let mut devices = Vec::new();

    for i in 0..objects.n_children() {
        let entry = objects.child_value(i);
        let Some(path) = entry.child_value(0).str().map(str::to_string) else {
            continue;
        };
        let ifaces = entry.child_value(1);

        let mut adapter_dict: Option<glib::VariantDict> = None;
        let mut dev_dict: Option<glib::VariantDict> = None;
        let mut bat_dict: Option<glib::VariantDict> = None;

        for j in 0..ifaces.n_children() {
            let iface_entry = ifaces.child_value(j);
            let iface_key = iface_entry.child_value(0);
            let Some(iface_name) = iface_key.str() else {
                continue;
            };
            let props = iface_entry.child_value(1);
            match iface_name {
                "org.bluez.Adapter1" => adapter_dict = Some(glib::VariantDict::new(Some(&props))),
                "org.bluez.Device1" => dev_dict = Some(glib::VariantDict::new(Some(&props))),
                "org.bluez.Battery1" => bat_dict = Some(glib::VariantDict::new(Some(&props))),
                _ => {}
            }
        }

        if let Some(ad) = adapter_dict {
            if adapter_path.is_none() {
                adapter_path = Some(path.clone());
                powered = ad.lookup::<bool>("Powered").ok().flatten().unwrap_or(false);
                discovering = ad.lookup::<bool>("Discovering").ok().flatten().unwrap_or(false);
            }
        }

        if let Some(dd) = dev_dict {
            let real_name = dd.lookup::<String>("Name").ok().flatten();
            let alias = dd.lookup::<String>("Alias").ok().flatten();
            let addr = dd.lookup::<String>("Address").ok().flatten().unwrap_or_default();
            let connected = dd.lookup::<bool>("Connected").ok().flatten().unwrap_or(false);
            let paired = dd.lookup::<bool>("Paired").ok().flatten().unwrap_or(false);

            // Skip unnamed BLE beacons unless already paired/connected.
            if real_name.is_none() && !paired && !connected {
                continue;
            }

            let name = real_name.or(alias).unwrap_or(addr);
            let icon_s = dd.lookup::<String>("Icon").ok().flatten();
            let battery = bat_dict
                .and_then(|bd| bd.lookup::<u8>("Percentage").ok().flatten())
                .map(|p| format!("{p}%"));

            devices.push(BtDevice {
                path,
                name,
                connected,
                paired,
                icon: device_icon(icon_s.as_deref()),
                battery,
            });
        }
    }

    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| b.paired.cmp(&a.paired))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    BluezSnapshot {
        adapter: adapter_path,
        powered,
        discovering,
        devices,
    }
}

fn set_adapter_prop(adapter_path: &str, prop: &str, active: bool) -> bool {
    let Ok(bus) = sys_bus() else {
        return false;
    };
    let val = glib::Variant::from(active);
    let args = ("org.bluez.Adapter1", prop, val).to_variant();
    bus.call_sync(
        Some(BLUEZ_BUS),
        adapter_path,
        "org.freedesktop.DBus.Properties",
        "Set",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        3000,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

fn set_adapter_powered(adapter_path: &str, active: bool) -> bool {
    set_adapter_prop(adapter_path, "Powered", active)
}

fn connect_or_disconnect_device(dev: &BtDevice, adapter: Option<&str>) -> Result<(), String> {
    let bus = sys_bus().map_err(|e| e.to_string())?;

    if dev.connected {
        bus.call_sync(
            Some(BLUEZ_BUS),
            &dev.path,
            "org.bluez.Device1",
            "Disconnect",
            None,
            None,
            gio::DBusCallFlags::NONE,
            10000,
            gio::Cancellable::NONE,
        )
        .map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Stop active discovery so Intel BR/EDR paging does not collide with scan.
    if let Some(ad) = adapter {
        let _ = bus.call_sync(
            Some(BLUEZ_BUS),
            ad,
            "org.bluez.Adapter1",
            "StopDiscovery",
            None,
            None,
            gio::DBusCallFlags::NONE,
            1500,
            gio::Cancellable::NONE,
        );
        if !dev.paired {
            set_adapter_prop(ad, "Pairable", true);
        }
    }

    let reg_id = register_temp_agent(&bus);

    if !dev.paired {
        let pair_res = bus.call_sync(
            Some(BLUEZ_BUS),
            &dev.path,
            "org.bluez.Device1",
            "Pair",
            None,
            None,
            gio::DBusCallFlags::NONE,
            20000,
            gio::Cancellable::NONE,
        );
        if let Err(err) = pair_res {
            unregister_temp_agent(&bus, reg_id);
            return Err(err.to_string());
        }

        let trust_args =
            ("org.bluez.Device1", "Trusted", glib::Variant::from(true)).to_variant();
        let _ = bus.call_sync(
            Some(BLUEZ_BUS),
            &dev.path,
            "org.freedesktop.DBus.Properties",
            "Set",
            Some(&trust_args),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        );
    }

    let conn_res = bus.call_sync(
        Some(BLUEZ_BUS),
        &dev.path,
        "org.bluez.Device1",
        "Connect",
        None,
        None,
        gio::DBusCallFlags::NONE,
        15000,
        gio::Cancellable::NONE,
    );
    unregister_temp_agent(&bus, reg_id);
    conn_res.map_err(|e| e.to_string())?;
    Ok(())
}

fn register_temp_agent(bus: &gio::DBusConnection) -> Option<gio::RegistrationId> {
    let node = gio::DBusNodeInfo::for_xml(AGENT_XML).ok()?;
    let iface = node.lookup_interface("org.bluez.Agent1")?;
    let reg_id = bus
        .register_object(
            AGENT_PATH,
            &iface,
            |_conn, _sender, _path, _iface, method, _params, inv| match method {
                "RequestPinCode" => inv.return_value(Some(&("0000",).to_variant())),
                "RequestPasskey" => inv.return_value(Some(&(0u32,).to_variant())),
                _ => inv.return_value(None),
            },
            |_conn, _sender, _path, _iface, _prop| false.to_variant(),
            |_conn, _sender, _path, _iface, _prop, _val| false,
        )
        .ok()?;

    let reg_args = (glib::variant::ObjectPath::try_from(AGENT_PATH).ok()?, "NoInputNoOutput")
        .to_variant();
    let _ = bus.call_sync(
        Some(BLUEZ_BUS),
        "/org/bluez",
        "org.bluez.AgentManager1",
        "RegisterAgent",
        Some(&reg_args),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    );
    let def_args = (glib::variant::ObjectPath::try_from(AGENT_PATH).ok()?,).to_variant();
    let _ = bus.call_sync(
        Some(BLUEZ_BUS),
        "/org/bluez",
        "org.bluez.AgentManager1",
        "RequestDefaultAgent",
        Some(&def_args),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    );
    Some(reg_id)
}

fn unregister_temp_agent(bus: &gio::DBusConnection, reg_id: Option<gio::RegistrationId>) {
    if let Ok(obj_path) = glib::variant::ObjectPath::try_from(AGENT_PATH) {
        let args = (obj_path,).to_variant();
        let _ = bus.call_sync(
            Some(BLUEZ_BUS),
            "/org/bluez",
            "org.bluez.AgentManager1",
            "UnregisterAgent",
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
        );
    }
    if let Some(id) = reg_id {
        let _ = bus.unregister_object(id);
    }
}

fn toggle_discovery(adapter_path: &str, start: bool) -> bool {
    let Ok(bus) = sys_bus() else {
        return false;
    };
    if start {
        let dict = glib::VariantDict::new(None);
        dict.insert("Transport", "auto");
        let filter = (dict.end(),).to_variant();
        let _ = bus.call_sync(
            Some(BLUEZ_BUS),
            adapter_path,
            "org.bluez.Adapter1",
            "SetDiscoveryFilter",
            Some(&filter),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        );
    }
    bus.call_sync(
        Some(BLUEZ_BUS),
        adapter_path,
        "org.bluez.Adapter1",
        if start { "StartDiscovery" } else { "StopDiscovery" },
        None,
        None,
        gio::DBusCallFlags::NONE,
        3000,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

#[derive(Clone)]
struct BtCtx {
    win: gtk::Window,
    scan_btn: gtk::Button,
    bt_switch: gtk::Switch,
    status_lbl: gtk::Label,
    list_box: gtk::Box,
    adapter: Rc<RefCell<Option<String>>>,
    discovering: Rc<Cell<bool>>,
    syncing: Rc<Cell<bool>>,
    busy: Rc<Cell<bool>>,
    auto_scanned: Rc<Cell<bool>>,
    scan_timer: Rc<RefCell<Option<glib::SourceId>>>,
    poll_timer: Rc<RefCell<Option<glib::SourceId>>>,
}

fn stop_scan_timers(ctx: &BtCtx) {
    if let Some(id) = ctx.scan_timer.borrow_mut().take() {
        id.remove();
    }
    if let Some(id) = ctx.poll_timer.borrow_mut().take() {
        id.remove();
    }
}

fn start_discovery_cycle(ctx: &BtCtx, adapter: String) {
    ctx.status_lbl.set_text("Поиск устройств...");
    let ctx_c = ctx.clone();
    let ad_start = adapter;
    run_bg(
        move || toggle_discovery(&ad_start, true),
        move |_| refresh(&ctx_c, false),
    );

    stop_scan_timers(ctx);
    let ctx_poll = ctx.clone();
    *ctx.poll_timer.borrow_mut() = Some(glib::timeout_add_local(
        Duration::from_millis(1200),
        move || {
            refresh(&ctx_poll, false);
            ControlFlow::Continue
        },
    ));

    let ctx_stop = ctx.clone();
    *ctx.scan_timer.borrow_mut() = Some(glib::timeout_add_local(
        Duration::from_secs(10),
        move || {
            *ctx_stop.scan_timer.borrow_mut() = None;
            if let Some(pid) = ctx_stop.poll_timer.borrow_mut().take() {
                pid.remove();
            }
            if let Some(ad) = ctx_stop.adapter.borrow().clone() {
                let ctx_after = ctx_stop.clone();
                run_bg(
                    move || toggle_discovery(&ad, false),
                    move |_| refresh(&ctx_after, false),
                );
            }
            ControlFlow::Break
        },
    ));
}

fn refresh(ctx: &BtCtx, allow_auto_scan: bool) {
    let ctx_c = ctx.clone();
    run_bg(get_bluez_state, move |snap| {
        *ctx_c.adapter.borrow_mut() = snap.adapter.clone();
        ctx_c.discovering.set(snap.discovering);
        ctx_c.syncing.set(true);
        ctx_c.bt_switch.set_active(snap.powered);
        ctx_c.bt_switch.set_state(snap.powered);
        ctx_c.syncing.set(false);
        ctx_c
            .scan_btn
            .set_sensitive(snap.adapter.is_some() && snap.powered);
        render(&ctx_c, &snap);

        if allow_auto_scan
            && !ctx_c.auto_scanned.get()
            && snap.powered
            && !snap.discovering
            && !snap.devices.iter().any(|d| d.paired || d.connected)
        {
            if let Some(ad) = snap.adapter {
                ctx_c.auto_scanned.set(true);
                start_discovery_cycle(&ctx_c, ad);
            }
        }
    });
}

fn toggle_device(ctx: &BtCtx, dev: BtDevice) {
    if ctx.busy.get() {
        return;
    }
    stop_scan_timers(ctx);
    ctx.busy.set(true);
    let verb = if dev.connected {
        "Отключение"
    } else if dev.paired {
        "Подключение"
    } else {
        "Сопряжение"
    };
    ctx.status_lbl.set_text(&format!("{verb}: {}...", dev.name));

    let ctx_c = ctx.clone();
    let adapter = ctx.adapter.borrow().clone();
    run_bg(
        move || connect_or_disconnect_device(&dev, adapter.as_deref()),
        move |res| {
            ctx_c.busy.set(false);
            if let Err(err) = res {
                let short = err.split(':').next_back().unwrap_or(&err).trim();
                ctx_c
                    .status_lbl
                    .set_text(&format!("Ошибка: {short}"));
            }
            refresh(&ctx_c, false);
        },
    );
}

fn render(ctx: &BtCtx, snap: &BluezSnapshot) {
    clear_container(&ctx.list_box);

    if snap.adapter.is_none() {
        ctx.status_lbl.set_text("Bluetooth адаптер не найден");
        ctx.win.show_all();
        return;
    }

    if !snap.powered {
        ctx.status_lbl.set_text("Bluetooth выключен");
        let lbl = gtk::Label::new(Some("Включите переключатель сверху"));
        lbl.style_context().add_class("muted-text");
        ctx.list_box.pack_start(&lbl, false, false, 12);
        ctx.win.show_all();
        return;
    }

    if !ctx.busy.get() {
        if snap.discovering {
            ctx.status_lbl.set_text("Поиск устройств...");
        } else {
            let cnt = snap.devices.iter().filter(|d| d.connected).count();
            ctx.status_lbl.set_text(&if cnt > 0 {
                format!("Подключено: {cnt}")
            } else {
                "Готов к подключению".to_string()
            });
        }
    }

    if snap.devices.is_empty() {
        let lbl = gtk::Label::new(Some("Устройства не найдены"));
        lbl.style_context().add_class("muted-text");
        ctx.list_box.pack_start(&lbl, false, false, 12);
    }

    for dev in &snap.devices {
        let btn = gtk::Button::new();
        btn.style_context().add_class("row-btn");
        if dev.connected {
            btn.style_context().add_class("active");
        }

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let icon = if dev.connected { "󰂱" } else { dev.icon };
        let name_lbl = gtk::Label::new(Some(&format!("{icon} {}", dev.name)));
        name_lbl.set_xalign(0.0);
        name_lbl.set_ellipsize(EllipsizeMode::End);

        let right_text = dev.battery.clone().unwrap_or_else(|| {
            if dev.connected {
                "Подкл.".into()
            } else if dev.paired {
                "Сопряж.".into()
            } else {
                "Новое".into()
            }
        });
        let meta_lbl = gtk::Label::new(Some(&right_text));
        meta_lbl
            .style_context()
            .add_class(if dev.connected { "accent" } else { "muted-text" });

        row.pack_start(&name_lbl, true, true, 0);
        row.pack_end(&meta_lbl, false, false, 0);
        btn.add(&row);

        let ctx_c = ctx.clone();
        let dev_c = dev.clone();
        btn.connect_clicked(move |_| toggle_device(&ctx_c, dev_c.clone()));
        ctx.list_box.pack_start(&btn, false, false, 0);
    }

    ctx.win.show_all();
}

pub fn build() -> gtk::Window {
    let popup = WidgetPopup::new("Bluetooth", "󰂯", Align::Right, 320);

    let scan_btn = gtk::Button::with_label("󰑓");
    scan_btn.style_context().add_class("icon-btn");
    popup.header_actions.pack_start(&scan_btn, false, false, 0);

    let bt_switch = gtk::Switch::new();
    bt_switch.set_valign(gtk::Align::Center);
    popup.header_actions.pack_start(&bt_switch, false, false, 0);

    let status_lbl = gtk::Label::new(None);
    status_lbl.style_context().add_class("widget-subtitle");
    status_lbl.set_xalign(0.0);
    popup.root_box.pack_start(&status_lbl, false, false, 0);

    popup
        .root_box
        .pack_start(&make_section_label("Устройства"), false, false, 0);

    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_min_content_height(140);
    scroll.set_max_content_height(220);
    let list_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    scroll.add(&list_box);
    popup.root_box.pack_start(&scroll, true, true, 0);

    let ctx = BtCtx {
        win: popup.window.clone(),
        scan_btn: scan_btn.clone(),
        bt_switch: bt_switch.clone(),
        status_lbl,
        list_box,
        adapter: Rc::new(RefCell::new(None)),
        discovering: Rc::new(Cell::new(false)),
        syncing: Rc::new(Cell::new(false)),
        busy: Rc::new(Cell::new(false)),
        auto_scanned: Rc::new(Cell::new(false)),
        scan_timer: Rc::new(RefCell::new(None)),
        poll_timer: Rc::new(RefCell::new(None)),
    };

    let ctx_dest = ctx.clone();
    popup.window.connect_destroy(move |_| {
        stop_scan_timers(&ctx_dest);
        if ctx_dest.discovering.get() {
            if let Some(ad) = ctx_dest.adapter.borrow().clone() {
                std::thread::spawn(move || {
                    toggle_discovery(&ad, false);
                });
            }
        }
    });

    let ctx_scan = ctx.clone();
    scan_btn.connect_clicked(move |_| {
        let Some(adapter) = ctx_scan.adapter.borrow().clone() else {
            return;
        };
        if ctx_scan.discovering.get() {
            stop_scan_timers(&ctx_scan);
            let ctx_c = ctx_scan.clone();
            run_bg(
                move || toggle_discovery(&adapter, false),
                move |_| refresh(&ctx_c, false),
            );
            return;
        }
        start_discovery_cycle(&ctx_scan, adapter);
    });

    let ctx_sw = ctx.clone();
    bt_switch.connect_state_set(move |_, active| {
        if ctx_sw.syncing.get() {
            return glib::Propagation::Proceed;
        }
        let Some(adapter) = ctx_sw.adapter.borrow().clone() else {
            return glib::Propagation::Proceed;
        };
        stop_scan_timers(&ctx_sw);
        ctx_sw
            .status_lbl
            .set_text(if active { "Включение..." } else { "Выключение..." });
        let ctx_c = ctx_sw.clone();
        run_bg(
            move || set_adapter_powered(&adapter, active),
            move |_| {
                let ctx_after = ctx_c.clone();
                glib::timeout_add_local(Duration::from_millis(250), move || {
                    refresh(&ctx_after, active);
                    ControlFlow::Break
                });
            },
        );
        glib::Propagation::Stop
    });

    refresh(&ctx, true);
    popup.window
}
