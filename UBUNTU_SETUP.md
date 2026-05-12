# Ubuntu系统下NAT Server部署指南

## 🐧 系统要求
- Ubuntu 20.04 LTS 或更高版本
- 至少 2GB RAM
- 1GB 可用磁盘空间
- 网络连接

## 📦 安装依赖

### 1. 更新系统包
```bash
sudo apt update && sudo apt upgrade -y
```

### 2. 安装Rust工具链
```bash
# 安装Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# 验证安装
rustc --version
cargo --version
```

### 3. 安装系统依赖
```bash
# 安装编译工具和库
sudo apt install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    sqlite3 \
    git \
    curl \
    wget

# 安装网络工具
sudo apt install -y \
    net-tools \
    ufw \
    nginx  # 可选，用于反向代理
```

### 4. 安装Node.js (可选，用于前端开发)
```bash
curl -fsSL https://deb.nodesource.com/setup_18.x | sudo -E bash -
sudo apt install -y nodejs
```

## 🚀 部署步骤

### 1. 克隆项目
```bash
git clone <repository-url> nat-server
cd nat-server
```

### 2. 编译项目
```bash
# 确保Rust环境已加载
source ~/.cargo/env

# 编译发布版本
cargo build --release

# 编译可能需要5-10分钟，取决于系统性能
```

### 3. 配置防火墙
```bash
# 开放必要端口
sudo ufw allow 21115  # Rendezvous端口
sudo ufw allow 21116  # Relay端口  
sudo ufw allow 8080   # API端口
sudo ufw allow 443    # HTTPS端口（如果使用）

# 启用防火墙
sudo ufw enable
```

### 4. 创建系统服务
```bash
# 创建systemd服务文件
sudo tee /etc/systemd/system/nat-server.service > /dev/null <<EOF
[Unit]
Description=NAT Server Service
After=network.target

[Service]
Type=simple
User=natserver
WorkingDirectory=/opt/nat-server
ExecStart=/opt/nat-server/target/release/hbbs
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# 创建用户
sudo useradd -r -s /bin/false natserver

# 复制文件到系统目录
sudo mkdir -p /opt/nat-server
sudo cp -r target/release/* /opt/nat-server/
sudo chown -R natserver:natserver /opt/nat-server

# 启用并启动服务
sudo systemctl daemon-reload
sudo systemctl enable nat-server
sudo systemctl start nat-server
```

### 5. 验证安装
```bash
# 检查服务状态
sudo systemctl status nat-server

# 检查端口监听
sudo netstat -tlnp | grep -E ':(21115|21116|8080)'

# 测试API
curl http://localhost:8080/
```

## 🔧 配置选项

### 环境变量配置
```bash
# 创建配置文件
sudo tee /opt/nat-server/.env > /dev/null <<EOF
# 数据库路径
DATABASE_PATH=/opt/nat-server/db_v2.sqlite3

# JWT密钥（请生成新的密钥）
JWT_SECRET=$(openssl rand -hex 32)

# 日志级别
RUST_LOG=info

# 端口配置
RENDEZVOUS_PORT=21115
RELAY_PORT=21116
API_PORT=8080
EOF

# 生成JWT密钥
JWT_SECRET=$(openssl rand -hex 32)
echo "JWT_SECRET=${JWT_SECRET}" | sudo tee -a /opt/nat-server/.env
```

### 数据库初始化
```bash
# 数据库会在首次运行时自动创建
# 可以手动检查数据库
sqlite3 /opt/nat-server/db_v2.sqlite3 ".tables"
```

## 🌐 反向代理配置（可选）

### Nginx配置
```bash
sudo tee /etc/nginx/sites-available/nat-server > /dev/null <<EOF
server {
    listen 80;
    server_name your-domain.com;

    location / {
        proxy_pass http://localhost:8080;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }
}
EOF

# 启用站点
sudo ln -s /etc/nginx/sites-available/nat-server /etc/nginx/sites-enabled/
sudo nginx -t
sudo systemctl reload nginx
```

## 🔒 SSL证书配置（可选）

### 使用Let's Encrypt
```bash
# 安装Certbot
sudo apt install certbot python3-certbot-nginx

# 获取证书
sudo certbot --nginx -d your-domain.com

# 自动续期
sudo crontab -e
# 添加以下行：
# 0 12 * * * /usr/bin/certbot renew --quiet
```

## 📊 监控和日志

### 查看日志
```bash
# 系统日志
sudo journalctl -u nat-server -f

# 应用日志（如果配置了）
sudo tail -f /opt/nat-server/logs/app.log
```

### 监控脚本
```bash
# 创建监控脚本
sudo tee /usr/local/bin/nat-server-monitor > /dev/null <<'EOF'
#!/bin/bash

# 检查服务状态
if ! systemctl is-active --quiet nat-server; then
    echo "NAT Server服务未运行，正在重启..."
    sudo systemctl restart nat-server
fi

# 检查端口
if ! netstat -tlnp | grep -q :8080; then
    echo "API端口8080未监听"
fi

# 检查数据库
if [ ! -f /opt/nat-server/db_v2.sqlite3 ]; then
    echo "数据库文件不存在"
fi
EOF

chmod +x /usr/local/bin/nat-server-monitor

# 添加到crontab
echo "*/5 * * * * /usr/local/bin/nat-server-monitor" | sudo crontab -
```

## 🧪 测试部署

### API测试
```bash
# 测试注册
curl -X POST http://localhost:8080/api/register \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "email": "admin@example.com",
    "password": "admin123",
    "confirm_password": "admin123"
  }'

# 测试登录
curl -X POST http://localhost:8080/api/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "admin123"
  }'
```

### Web界面测试
```bash
# 访问Web界面
curl http://localhost:8080/
```

## 🔄 更新和维护

### 更新应用
```bash
cd /opt/nat-server
# 停止服务
sudo systemctl stop nat-server

# 备份当前版本
sudo cp -r target/release target/release.backup

# 更新代码
git pull origin main

# 重新编译
cargo build --release

# 替换二进制文件
sudo cp target/release/* /opt/nat-server/
sudo chown -R natserver:natserver /opt/nat-server

# 重启服务
sudo systemctl start nat-server
```

### 数据库备份
```bash
# 创建备份脚本
sudo tee /usr/local/bin/nat-server-backup > /dev/null <<'EOF'
#!/bin/bash

BACKUP_DIR="/opt/backups/nat-server"
DATE=$(date +%Y%m%d_%H%M%S)

mkdir -p $BACKUP_DIR

# 备份数据库
cp /opt/nat-server/db_v2.sqlite3 $BACKUP_DIR/db_v2_${DATE}.sqlite3

# 删除7天前的备份
find $BACKUP_DIR -name "db_v2_*.sqlite3" -mtime +7 -delete

echo "数据库备份完成: $BACKUP_DIR/db_v2_${DATE}.sqlite3"
EOF

chmod +x /usr/local/bin/nat-server-backup

# 添加到crontab（每日凌晨2点备份）
echo "0 2 * * * /usr/local/bin/nat-server-backup" | sudo crontab -
```

## 🚨 故障排除

### 常见问题

1. **编译失败**
   ```bash
   # 确保所有依赖已安装
   sudo apt install build-essential libssl-dev libsqlite3-dev
   
   # 清理并重新编译
   cargo clean
   cargo build --release
   ```

2. **端口被占用**
   ```bash
   # 查找占用端口的进程
   sudo lsof -i :8080
   
   # 终止进程
   sudo kill -9 <PID>
   ```

3. **权限问题**
   ```bash
   # 检查文件权限
   ls -la /opt/nat-server/
   
   # 修复权限
   sudo chown -R natserver:natserver /opt/nat-server/
   ```

4. **服务无法启动**
   ```bash
   # 查看详细错误
   sudo journalctl -u nat-server -n 50
   
   # 检查配置文件
   cat /opt/nat-server/.env
   ```

## 📞 技术支持

### 日志位置
- 系统日志: `/var/log/syslog`
- 服务日志: `journalctl -u nat-server`
- 应用日志: `/opt/nat-server/logs/`（如果配置）

### 配置文件位置
- 服务配置: `/etc/systemd/system/nat-server.service`
- 环境变量: `/opt/nat-server/.env`
- Nginx配置: `/etc/nginx/sites-available/nat-server`

### 性能优化
```bash
# 调整系统参数
echo 'net.core.somaxconn = 65535' | sudo tee -a /etc/sysctl.conf
echo 'net.ipv4.tcp_max_syn_backlog = 65535' | sudo tee -a /etc/sysctl.conf
sudo sysctl -p
```

## 🎯 部署检查清单

- [ ] 系统依赖已安装
- [ ] Rust工具链已配置
- [ ] 项目编译成功
- [ ] 防火墙规则已配置
- [ ] 系统服务已创建并启动
- [ ] 数据库已初始化
- [ ] API端点可访问
- [ ] Web界面正常显示
- [ ] 日志记录正常
- [ ] 备份策略已实施
- [ ] 监控脚本已配置

完成以上步骤后，您的NAT Server将在Ubuntu系统上稳定运行！
