#![allow(dead_code)]
use crate::config::{parse_hex_color, Config};
use crate::modules::{ClickAction, ClickRegion, ModuleBlock};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use zbus::{connection::Builder, interface, proxy, Connection};

#[derive(Debug, Clone)]
pub struct TrayItem {
    pub service: String,
    pub path: String,
    pub title: String,
    pub icon_width: u32,
    pub icon_height: u32,
    pub pixmap: Arc<Vec<u8>>, // ARGB 32-bit
}

#[derive(Clone)]
pub struct TrayModule {
    pub items: Arc<RwLock<HashMap<String, TrayItem>>>,
    connection: Arc<RwLock<Option<Connection>>>,
}

#[proxy(
    default_service = "org.kde.StatusNotifierItem",
    interface = "org.kde.StatusNotifierItem"
)]
trait StatusNotifierItem {
    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn scroll(&self, delta: i32, orientation: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;
}

struct WatcherServer {
    items: Arc<RwLock<HashMap<String, TrayItem>>>,
    conn: Connection,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl WatcherServer {
    async fn register_status_notifier_item(
        &mut self,
        service: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) {
        let actual_service = if service.starts_with('/') {
            if let Some(sender) = hdr.sender() {
                format!("{}{}", sender, service)
            } else {
                service.to_string()
            }
        } else {
            service.to_string()
        };

        let (bus_name, obj_path) = if let Some((bus, path)) = actual_service.split_once('/') {
            (bus.to_string(), format!("/{}", path))
        } else {
            (actual_service.clone(), "/StatusNotifierItem".to_string())
        };

        println!("[tray] Registered SNI item: bus='{}', path='{}'", bus_name, obj_path);

        let items_store = self.items.clone();
        let conn_clone = self.conn.clone();
        let service_key = actual_service.clone();

        tokio::spawn(async move {
            if let Ok(proxy) = StatusNotifierItemProxy::builder(&conn_clone)
                .destination(bus_name)
                .path(obj_path.clone())
                .build()
                .await
            {
                let title = proxy.title().await.unwrap_or_else(|_| "Tray Item".to_string());
                let mut best_pixmap: Option<(u32, u32, Vec<u8>)> = None;

                    if let Ok(pixmaps) = proxy.icon_pixmap().await {
                        for (w, h, data) in pixmaps {
                            if w > 0 && h > 0 && data.len() >= (w * h * 4) as usize {
                                let diff = (w - 22).abs();
                                let is_better = match &best_pixmap {
                                    None => true,
                                    Some((bw, _, _)) => (bw.cast_signed() - 22).abs() > diff,
                                };
                                if is_better {
                                    best_pixmap = Some((w as u32, h as u32, data));
                                }
                            }
                        }
                    }

                    let (w, h, pix) = best_pixmap.unwrap_or_else(|| {
                        let mut dummy = vec![0u8; 16 * 16 * 4];
                        for y in 0..16 {
                            for x in 0..16 {
                                if (x + y >= 6 && x + y <= 24) && (x as i32 - y as i32).abs() <= 8 {
                                    let idx = ((y * 16 + x) * 4) as usize;
                                    dummy[idx] = 255;
                                    dummy[idx + 1] = 137;
                                    dummy[idx + 2] = 180;
                                    dummy[idx + 3] = 250;
                                }
                            }
                        }
                        (16, 16, dummy)
                    });

                    let item = TrayItem {
                        service: service_key.clone(),
                        path: obj_path,
                        title,
                        icon_width: w,
                        icon_height: h,
                        pixmap: Arc::new(pix),
                    };

                    let mut map = items_store.write().await;
                    map.insert(service_key, item);
                }
        });
    }

    async fn register_status_notifier_host(&self, _service: &str) {
        println!("[tray] StatusNotifierHost registered");
    }

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        let map = self.items.read().await;
        map.keys().cloned().collect()
    }

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }
}

impl TrayModule {
    pub fn new() -> Self {
        let items = Arc::new(RwLock::new(HashMap::new()));
        let connection = Arc::new(RwLock::new(None));

        let module = Self {
            items: items.clone(),
            connection: connection.clone(),
        };

        module.spawn_dbus_watcher(items, connection);
        module
    }

    fn spawn_dbus_watcher(&self, items: Arc<RwLock<HashMap<String, TrayItem>>>, conn_store: Arc<RwLock<Option<Connection>>>) {
        tokio::spawn(async move {
            let session_conn = match Connection::session().await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[tray] Failed to connect to session bus: {}", e);
                    return;
                }
            };

            let watcher = WatcherServer {
                items: items.clone(),
                conn: session_conn.clone(),
            };

            let builder = Builder::session()
                .name("org.kde.StatusNotifierWatcher")
                .serve_at("/StatusNotifierWatcher", watcher);

            if let Ok(connection) = builder.build().await {
                println!("[tray] StatusNotifierWatcher active on DBus");
                {
                    let mut g = conn_store.write().await;
                    *g = Some(connection.clone());
                }
                std::future::pending::<()>().await;
            }
        });
    }

    pub async fn activate_item(&self, service: &str, x: i32, y: i32) {
        let (bus_name, obj_path) = if let Some((b, p)) = service.split_once('/') {
            (b.to_string(), format!("/{}", p))
        } else {
            (service.to_string(), "/StatusNotifierItem".to_string())
        };

        let conn = {
            let g = self.connection.read().await;
            g.clone()
        };

        if let Some(c) = conn {
            if let Ok(proxy) = StatusNotifierItemProxy::builder(&c)
                .destination(bus_name)
                .path(obj_path)
                .build()
                .await
            {
                let _ = proxy.activate(x, y).await;
            }
        }
    }

    pub async fn context_menu_item(&self, service: &str, x: i32, y: i32) {
        let (bus_name, obj_path) = if let Some((b, p)) = service.split_once('/') {
            (b.to_string(), format!("/{}", p))
        } else {
            (service.to_string(), "/StatusNotifierItem".to_string())
        };

        let conn = {
            let g = self.connection.read().await;
            g.clone()
        };

        if let Some(c) = conn {
            if let Ok(proxy) = StatusNotifierItemProxy::builder(&c)
                .destination(bus_name)
                .path(obj_path)
                .build()
                .await
            {
                let _ = proxy.context_menu(x, y).await;
            }
        }
    }

    pub fn render(&self, config: &Config) -> ModuleBlock {
        let items_map = {
            if let Ok(guard) = self.items.try_read() {
                guard.clone()
            } else {
                HashMap::new()
            }
        };

        let fg = parse_hex_color(&config.colors.foreground);
        let bg = parse_hex_color(&config.colors.block_background);

        let mut block = ModuleBlock::new("tray", "", fg, bg);

        if items_map.is_empty() {
            return block;
        }

        for (service, item) in items_map {
            block.tray_pixmap = Some((item.icon_width, item.icon_height, item.pixmap.clone()));
            block.click_regions.push(ClickRegion {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
                action: ClickAction::TrayActivate(service.clone()),
            });
        }

        block
    }
}
