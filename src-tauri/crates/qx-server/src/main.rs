//! qx-server 二进制：`serve` 起边缘；`admin` 管实例/设备（仅回环语义：在哪台机器上跑就管哪个库）。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use qx_server::db::Db;
use qx_server::governance::GovConfig;
use qx_server::hub::Hub;
use qx_server::web::{self, EdgeState};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("serve") => run_serve(&args[1..]),
        Some("admin") => run_admin(&args[1..]),
        _ => {
            print_usage();
            0
        }
    };
    std::process::exit(code);
}

fn print_usage() {
    println!(
        "qx-server {} —— 千寻服务端（docs/19）

用法：
  qx-server serve [--bind 127.0.0.1:23100] [--data DIR] --public-host HOST [--public-proto https]
  qx-server admin add-instance    --data DIR --name NAME [--id ID]
  qx-server admin list-instances  --data DIR
  qx-server admin rotate-token    --data DIR --id ID
  qx-server admin pair-create     --data DIR
  qx-server admin import-device   --data DIR --name NAME [--token TOKEN]
  qx-server admin add-user        --data DIR --name NAME [--password PASS]
  qx-server admin list-users      --data DIR
  qx-server admin reset-password  --data DIR --name NAME [--password PASS]
  qx-server admin rename-user     --data DIR --from OLD --to NEW",
        qx_server::version()
    );
}

struct Flags {
    bind: SocketAddr,
    data: PathBuf,
    public_host: Option<String>,
    public_proto: &'static str,
}

fn parse_serve(args: &[String]) -> Result<Flags, String> {
    let mut flags = Flags {
        bind: "127.0.0.1:23100".parse().expect("默认值合法"),
        data: dirs_fallback_data()?,
        public_host: None,
        public_proto: "https",
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--bind" => {
                index += 1;
                flags.bind = args
                    .get(index)
                    .ok_or("--bind 需要值")?
                    .parse()
                    .map_err(|e| format!("--bind 不合法：{e}"))?;
            }
            "--data" => {
                index += 1;
                flags.data = PathBuf::from(args.get(index).ok_or("--data 需要值")?);
            }
            "--public-host" => {
                index += 1;
                flags.public_host = Some(args.get(index).ok_or("--public-host 需要值")?.to_owned());
            }
            "--public-proto" => {
                index += 1;
                match args.get(index).map(String::as_str) {
                    Some("https") => flags.public_proto = "https",
                    Some("http") => flags.public_proto = "http",
                    _ => return Err("--public-proto 只支持 http/https".into()),
                }
            }
            other => return Err(format!("未知参数：{other}")),
        }
        index += 1;
    }
    Ok(flags)
}

fn dirs_fallback_data() -> Result<PathBuf, String> {
    // 不引 dirs 依赖：默认数据目录 = ./qx-server-data（部署文档要求显式 --data 到 /var/lib）。
    Ok(PathBuf::from("qx-server-data"))
}

fn run_serve(args: &[String]) -> i32 {
    let flags = match parse_serve(args) {
        Ok(flags) => flags,
        Err(cause) => {
            eprintln!("{cause}");
            return 2;
        }
    };
    let Some(public_host) = flags.public_host.clone() else {
        eprintln!("--public-host 必填（Host 重写目标，docs/19 §4.3）");
        return 2;
    };
    let db = match Db::open(&flags.data.join("qx.db")) {
        Ok(db) => db,
        Err(cause) => {
            eprintln!("打开数据库失败：{cause}");
            return 1;
        }
    };
    let hub = Hub::new(Arc::new(db), GovConfig::default());
    let state = Arc::new(EdgeState {
        hub,
        public_host,
        public_proto: flags.public_proto,
        limiter: Arc::new(qx_server::web::AttemptLimiter::default()),
    });
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(cause) => {
            eprintln!("启动 tokio 失败：{cause}");
            return 1;
        }
    };
    // 没嵌进前端要在启动日志里说，而不是让人对着一个空控制台找原因
    // （/qx-pair 与 /qx-fleet 不受影响，它们是手写零构建页）。
    if !qx_server::assets::embedded() {
        eprintln!("注意：控制台资产未嵌入，/console 与 /login 返回 503；请先 `pnpm build:console` 再重编译 qx-server");
    }
    runtime.block_on(async move {
        let listener = match tokio::net::TcpListener::bind(flags.bind).await {
            Ok(listener) => listener,
            Err(cause) => {
                eprintln!("监听 {} 失败：{cause}", flags.bind);
                std::process::exit(1);
            }
        };
        println!("qx-server {} 监听 {}", qx_server::version(), flags.bind);
        if let Err(cause) = web::serve(listener, state).await {
            eprintln!("边缘退出：{cause}");
            std::process::exit(1);
        }
    });
    0
}

fn run_admin(args: &[String]) -> i32 {
    let Some(sub) = args.first().cloned() else {
        print_usage();
        return 2;
    };
    let rest = &args[1..];
    let data = flag_value(rest, "--data")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("qx-server-data"));
    let db = match Db::open(&data.join("qx.db")) {
        Ok(db) => db,
        Err(cause) => {
            eprintln!("打开数据库失败：{cause}");
            return 1;
        }
    };
    match sub.as_str() {
        "add-instance" => {
            let Some(name) = flag_value(rest, "--name") else {
                eprintln!("--name 必填");
                return 2;
            };
            let id = flag_value(rest, "--id")
                .unwrap_or_else(|| format!("pc-{}", qx_server::crypto::random_hex(4)));
            match db.add_instance(&id, &name) {
                Ok(token) => {
                    println!("实例已预置（fail-closed，无自动注册）");
                    println!("  instanceId     = {id}");
                    println!("  instanceToken  = {token}");
                    println!("在千寻远程页「服务器」区块填入以上两项与地址。");
                    0
                }
                Err(cause) => {
                    eprintln!("{cause}");
                    1
                }
            }
        }
        "list-instances" => {
            for row in db.instances() {
                println!("{}\t{}\tlast_seen={}", row.id, row.name, row.last_seen_at);
            }
            0
        }
        "rotate-token" => {
            let Some(id) = flag_value(rest, "--id") else {
                eprintln!("--id 必填");
                return 2;
            };
            match db.rotate_instance_token(&id) {
                Ok(token) => {
                    println!("新 instanceToken = {token}");
                    0
                }
                Err(cause) => {
                    eprintln!("{cause}");
                    1
                }
            }
        }
        "pair-create" => match db.create_pair_code("admin") {
            Ok((code, expires_at)) => {
                println!("配对码（5 分钟内一次性）：{code}");
                println!("expires_at = {expires_at}");
                0
            }
            Err(cause) => {
                eprintln!("{cause}");
                1
            }
        },
        "import-device" => {
            let Some(name) = flag_value(rest, "--name") else {
                eprintln!("--name 必填");
                return 2;
            };
            match db.import_device(&name, flag_value(rest, "--token").as_deref()) {
                Ok(token) => {
                    println!("设备已导入；token = {token}");
                    0
                }
                Err(cause) => {
                    eprintln!("{cause}");
                    1
                }
            }
        }
        "add-user" => {
            let Some(name) = flag_value(rest, "--name") else {
                eprintln!("--name 必填");
                return 2;
            };
            // 无默认密码：未显式给 --password 则随机生成并只打印一次。
            let password =
                flag_value(rest, "--password").unwrap_or_else(|| qx_server::crypto::random_hex(8));
            match db.add_user(&name, &password) {
                Ok(_id) => {
                    println!("管理用户已预置（/console 登录用）");
                    println!("  username = {name}");
                    println!("  password = {password}");
                    if flag_value(rest, "--password").is_none() {
                        println!("（密码为随机生成，只显示这一次；忘记可用 reset-password 重置）");
                    }
                    0
                }
                Err(cause) => {
                    eprintln!("{cause}");
                    1
                }
            }
        }
        "list-users" => {
            for (id, name, created) in db.users() {
                println!("{id}\t{name}\tcreated={created}");
            }
            0
        }
        "reset-password" => {
            let Some(name) = flag_value(rest, "--name") else {
                eprintln!("--name 必填");
                return 2;
            };
            let password =
                flag_value(rest, "--password").unwrap_or_else(|| qx_server::crypto::random_hex(8));
            match db.reset_password(&name, &password) {
                Ok(()) => {
                    println!("新 password = {password}（旧会话已全部吊销）");
                    0
                }
                Err(cause) => {
                    eprintln!("{cause}");
                    1
                }
            }
        }
        "rename-user" => {
            let (Some(old), Some(new)) = (flag_value(rest, "--from"), flag_value(rest, "--to"))
            else {
                eprintln!("--from 与 --to 均必填");
                return 2;
            };
            match db.rename_user(&old, &new) {
                Ok(()) => {
                    println!("{old} 已改名为 {new}（行 id 未变，已有会话继续有效）");
                    0
                }
                Err(cause) if cause == "unknown_user" => {
                    eprintln!("没有叫 {old} 的用户");
                    1
                }
                Err(cause) => {
                    eprintln!("{cause}");
                    1
                }
            }
        }
        other => {
            eprintln!("未知 admin 子命令：{other}");
            print_usage();
            2
        }
    }
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find_map(|pair| (pair[0] == name).then(|| pair[1].clone()))
}
