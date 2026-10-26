use std::{
    env, fs,
    io::ErrorKind,
    net::{ToSocketAddrs, UdpSocket},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

use rosc::{OscMessage, OscPacket, OscType};
use serde::{Serialize, de::DeserializeOwned};

const ANNOUNCE: &str = "/nsm/server/announce";
pub const OPEN: &str = "/nsm/client/open";
pub const SAVE: &str = "/nsm/client/save";

pub struct Session {
    socket: UdpSocket,
    pub directory: PathBuf,
    pub display_name: String,
    pub client_id: String,
    pub terminate: Arc<AtomicBool>,
}

impl Session {
    pub fn from_env() -> Option<Self> {
        let url = env::var_os("NSM_URL")?;
        let result = url
            .to_str()
            .ok_or_else(|| "NSM_URL is not valid UTF-8".to_owned())
            .and_then(Self::connect);
        match result {
            Ok(session) => Some(session),
            Err(error) => {
                log::warn!("NSM unavailable, starting standalone: {error}");
                None
            }
        }
    }

    fn connect(url: &str) -> Result<Self, String> {
        let address = url
            .strip_prefix("osc.udp://")
            .map(|address| address.trim_end_matches('/'))
            .filter(|address| !address.contains('/'))
            .ok_or("NSM_URL must be an osc.udp://host:port/ URL")?;
        let remote = address
            .to_socket_addrs()
            .map_err(|e| format!("NSM address: {e}"))?
            .next()
            .ok_or("NSM address resolved to no endpoints")?;
        let socket = UdpSocket::bind(if remote.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })
        .map_err(|e| format!("NSM socket: {e}"))?;
        socket
            .connect(remote)
            .map_err(|e| format!("NSM connection: {e}"))?;
        let mut session = Self {
            socket,
            directory: PathBuf::new(),
            display_name: String::new(),
            client_id: String::new(),
            terminate: Arc::new(AtomicBool::new(false)),
        };
        session.send(
            ANNOUNCE,
            vec![
                "Bytebeat".into(),
                "".into(),
                env::args().next().ok_or("Missing executable name")?.into(),
                OscType::Int(1),
                OscType::Int(0),
                OscType::Int(std::process::id() as i32),
            ],
        )?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut announced = false;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("NSM handshake timed out".into());
            }
            session
                .socket
                .set_read_timeout(Some(remaining))
                .map_err(|e| e.to_string())?;
            let message = session.receive()?.ok_or("NSM handshake timed out")?;
            match (message.addr.as_str(), message.args.as_slice()) {
                (
                    "/reply",
                    [
                        OscType::String(path),
                        OscType::String(_),
                        OscType::String(_),
                        OscType::String(_),
                    ],
                ) if path == ANNOUNCE => announced = true,
                (
                    "/error",
                    [
                        OscType::String(path),
                        OscType::Int(_),
                        OscType::String(error),
                    ],
                ) if path == ANNOUNCE => return Err(error.clone()),
                (
                    OPEN,
                    [
                        OscType::String(path),
                        OscType::String(name),
                        OscType::String(id),
                    ],
                ) if announced => {
                    session.directory = PathBuf::from(path);
                    if !session.directory.is_absolute() || id.is_empty() {
                        session.respond(
                            OPEN,
                            Err((-9, "Invalid NSM project path or client ID".into())),
                        )?;
                        return Err("Invalid NSM project path or client ID".into());
                    }
                    session.display_name.clone_from(name);
                    session.client_id.clone_from(id);
                    break;
                }
                _ => {}
            }
        }
        session
            .socket
            .set_nonblocking(true)
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        signal_hook::flag::register(signal_hook::consts::SIGTERM, session.terminate.clone())
            .map_err(|e| format!("NSM SIGTERM handler: {e}"))?;
        log::info!(
            "NSM session: {} ({})",
            session.display_name,
            session.client_id
        );
        Ok(session)
    }

    fn send(&self, address: &str, args: Vec<OscType>) -> Result<(), String> {
        let packet = OscPacket::Message(OscMessage {
            addr: address.into(),
            args,
        });
        let data = rosc::encoder::encode(&packet).map_err(|e| format!("NSM OSC encode: {e}"))?;
        self.socket
            .send(&data)
            .map_err(|e| format!("NSM send: {e}"))?;
        Ok(())
    }

    pub fn receive(&self) -> Result<Option<OscMessage>, String> {
        let mut data = [0; 65535];
        let size = match self.socket.recv(&mut data) {
            Ok(size) => size,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Ok(None);
            }
            Err(e) => return Err(format!("NSM receive: {e}")),
        };
        match rosc::decoder::decode_udp(&data[..size]) {
            Ok(([], OscPacket::Message(message))) => Ok(Some(message)),
            Ok(_) => Err("NSM expected a single OSC message".into()),
            Err(e) => Err(format!("NSM OSC decode: {e}")),
        }
    }

    pub fn respond(&self, path: &str, result: Result<(), (i32, String)>) -> Result<(), String> {
        match result {
            Ok(()) => self.send("/reply", vec![path.into(), "OK".into()]),
            Err((code, error)) => {
                log::error!("NSM {path}: {error}");
                self.send(
                    "/error",
                    vec![path.into(), OscType::Int(code), error.into()],
                )
            }
        }
    }

    pub fn open<T: DeserializeOwned + Serialize + Default>(&self) -> Result<T, (i32, String)> {
        let path = self.directory.join("project.json");
        match fs::read(&path) {
            Ok(data) => serde_json::from_slice(&data)
                .map_err(|e| (-9, format!("Reading {}: {e}", path.display()))),
            Err(e) if e.kind() == ErrorKind::NotFound && !self.directory.exists() => {
                let project = T::default();
                self.save(&project).map_err(|error| (-10, error))?;
                Ok(project)
            }
            Err(e) => Err((-9, format!("Reading {}: {e}", path.display()))),
        }
    }

    pub fn save<T: Serialize>(&self, project: &T) -> Result<(), String> {
        let save = || -> Result<(), Box<dyn std::error::Error>> {
            fs::create_dir_all(&self.directory)?;
            let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
            serde_json::to_writer_pretty(file.as_file_mut(), project)?;
            file.as_file().sync_all()?;
            file.persist(self.directory.join("project.json"))?;
            Ok(())
        };
        save().map_err(|e| format!("Saving NSM project in {}: {e}", self.directory.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn receive(socket: &UdpSocket) -> (OscMessage, std::net::SocketAddr) {
        let mut buffer = [0; 4096];
        let (size, address) = socket.recv_from(&mut buffer).unwrap();
        let (_, OscPacket::Message(message)) = rosc::decoder::decode_udp(&buffer[..size]).unwrap()
        else {
            panic!("Expected OSC message")
        };
        (message, address)
    }

    fn send(socket: &UdpSocket, address: std::net::SocketAddr, path: &str, args: Vec<OscType>) {
        let data = rosc::encoder::encode(&OscPacket::Message(OscMessage {
            addr: path.into(),
            args,
        }))
        .unwrap();
        socket.send_to(&data, address).unwrap();
    }

    fn session(directory: PathBuf) -> Session {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.connect(socket.local_addr().unwrap()).unwrap();
        Session {
            socket,
            directory,
            display_name: "Test session".into(),
            client_id: "Bytebeat.test".into(),
            terminate: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn announces_opens_and_saves_using_the_same_socket() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("Bytebeat.test");
        let project_path = directory.clone();
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let url = format!("osc.udp://{}/", server.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (message, peer) = receive(&server);
            assert_eq!(message.addr, ANNOUNCE);
            assert!(matches!(message.args.as_slice(), [
                OscType::String(name), OscType::String(capabilities), OscType::String(executable),
                OscType::Int(1), OscType::Int(0), OscType::Int(_)
            ] if name == "Bytebeat" && capabilities.is_empty() && !executable.is_empty()));
            send(
                &server,
                peer,
                "/reply",
                vec![
                    ANNOUNCE.into(),
                    "Welcome".into(),
                    "NSM".into(),
                    ":server-control:".into(),
                ],
            );
            send(
                &server,
                peer,
                OPEN,
                vec![
                    project_path.to_str().unwrap().into(),
                    "Test session".into(),
                    "Bytebeat.test".into(),
                ],
            );
            let (opened, sender) = receive(&server);
            assert_eq!(sender, peer);
            assert_eq!(opened.addr, "/reply");
            assert_eq!(opened.args, vec![OPEN.into(), "OK".into()]);
            send(&server, peer, SAVE, vec![]);
            let (saved, sender) = receive(&server);
            assert_eq!(sender, peer);
            assert_eq!(saved.addr, "/reply");
            assert_eq!(saved.args, vec![SAVE.into(), "OK".into()]);
        });
        let session = Session::connect(&url).unwrap();
        assert_eq!(session.directory, directory);
        assert_eq!(session.client_id, "Bytebeat.test");
        assert_eq!(session.open::<Value>().unwrap(), Value::Null);
        let project = json!({"code": "t>>4", "rate": 35498.66666, "saved": []});
        session.save(&project).unwrap();
        session.respond(OPEN, Ok(())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let message = loop {
            if let Some(message) = session.receive().unwrap() {
                break message;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(message.addr, SAVE);
        session.save(&project).unwrap();
        session.respond(SAVE, Ok(())).unwrap();
        worker.join().unwrap();
        assert_eq!(session.open::<Value>().unwrap(), project);
    }

    #[test]
    fn saving_replaces_project_atomically_without_leaving_temporary_files() {
        let root = tempfile::tempdir().unwrap();
        let session = session(root.path().join("Bytebeat.test"));
        session.save(&json!({"code": "old"})).unwrap();
        session.save(&json!({"code": "new"})).unwrap();
        assert_eq!(session.open::<Value>().unwrap(), json!({"code": "new"}));
        assert_eq!(fs::read_dir(&session.directory).unwrap().count(), 1);
    }

    #[test]
    fn corrupt_or_incomplete_projects_are_not_silently_reset() {
        let root = tempfile::tempdir().unwrap();
        let session = session(root.path().to_owned());
        assert!(session.open::<Value>().is_err());
        fs::write(root.path().join("project.json"), "not json").unwrap();
        assert!(session.open::<Value>().is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("project.json")).unwrap(),
            "not json"
        );
    }

    #[test]
    fn saving_errors_are_reported_to_the_manager() {
        let root = tempfile::tempdir().unwrap();
        let session = session(root.path().join("not-a-directory"));
        fs::write(&session.directory, "file").unwrap();
        let error = session.save(&json!({"code": "t"})).unwrap_err();
        session.respond(SAVE, Err((-1, error.clone()))).unwrap();
        let message = session.receive().unwrap().unwrap();
        assert_eq!(message.addr, "/error");
        assert_eq!(
            message.args,
            vec![SAVE.into(), OscType::Int(-1), error.into()]
        );
    }

    #[test]
    fn opening_existing_projects_does_not_rewrite_them() {
        let root = tempfile::tempdir().unwrap();
        let session = session(root.path().to_owned());
        let path = root.path().join("project.json");
        let contents = "{\"code\":\"t\", \"rate\":8000}\n";
        fs::write(&path, contents).unwrap();
        let original = fs::metadata(&path).unwrap().permissions();
        let mut permissions = original.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        assert_eq!(
            session.open::<Value>().unwrap(),
            json!({"code": "t", "rate": 8000})
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        fs::set_permissions(&path, original).unwrap();
    }

    #[test]
    fn invalid_or_unsupported_urls_are_rejected() {
        for url in [
            "",
            "http://localhost:1234",
            "osc.tcp://localhost:1234/",
            "osc.udp://localhost:1234/extra",
            "osc.udp://localhost/",
        ] {
            assert!(Session::connect(url).is_err(), "{url}");
        }
    }

    #[test]
    fn rejected_announcements_do_not_open_projects() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let url = format!("osc.udp://{}/", server.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (_, peer) = receive(&server);
            send(
                &server,
                peer,
                "/error",
                vec![ANNOUNCE.into(), OscType::Int(-2), "Incompatible API".into()],
            );
        });
        assert!(
            Session::connect(&url)
                .err()
                .unwrap()
                .contains("Incompatible API")
        );
        worker.join().unwrap();
    }
}
