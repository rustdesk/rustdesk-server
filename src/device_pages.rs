// 设备管理页面
use axum::{
    response::{Html, IntoResponse},
};

pub async fn dashboard_page() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>控制台 - NAT Server</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }
        
        body {
            font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
            background: #f5f5f5;
            color: #333;
        }
        
        .header {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 1rem 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }
        
        .header h1 {
            font-size: 1.5rem;
        }
        
        .user-info {
            display: flex;
            align-items: center;
            gap: 1rem;
        }
        
        .logout-btn {
            background: rgba(255, 255, 255, 0.2);
            border: 1px solid rgba(255, 255, 255, 0.3);
            color: white;
            padding: 0.5rem 1rem;
            border-radius: 5px;
            cursor: pointer;
            transition: background 0.3s;
        }
        
        .logout-btn:hover {
            background: rgba(255, 255, 255, 0.3);
        }
        
        .container {
            max-width: 1200px;
            margin: 2rem auto;
            padding: 0 1rem;
        }
        
        .dashboard-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 2rem;
            margin-bottom: 2rem;
        }
        
        .card {
            background: white;
            padding: 1.5rem;
            border-radius: 10px;
            box-shadow: 0 2px 10px rgba(0, 0, 0, 0.1);
            transition: transform 0.2s;
        }
        
        .card:hover {
            transform: translateY(-2px);
        }
        
        .card h3 {
            color: #667eea;
            margin-bottom: 1rem;
            font-size: 1.2rem;
        }
        
        .card p {
            color: #666;
            margin-bottom: 1rem;
        }
        
        .card-btn {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            border: none;
            padding: 0.75rem 1.5rem;
            border-radius: 5px;
            cursor: pointer;
            transition: transform 0.2s;
        }
        
        .card-btn:hover {
            transform: translateY(-2px);
        }
        
        .stats-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 1rem;
            margin-bottom: 2rem;
        }
        
        .stat-card {
            background: white;
            padding: 1rem;
            border-radius: 10px;
            box-shadow: 0 2px 10px rgba(0, 0, 0, 0.1);
            text-align: center;
        }
        
        .stat-number {
            font-size: 2rem;
            font-weight: bold;
            color: #667eea;
        }
        
        .stat-label {
            color: #666;
            font-size: 0.9rem;
            margin-top: 0.5rem;
        }
        
        .error-message {
            background: #fee;
            color: #c33;
            padding: 0.75rem;
            border-radius: 5px;
            margin-bottom: 1rem;
            display: none;
        }
    </style>
</head>
<body>
    <div class="header">
        <h1>NAT Server 控制台</h1>
        <div class="user-info">
            <span id="username">加载中...</span>
            <button class="logout-btn" onclick="logout()">退出</button>
        </div>
    </div>
    
    <div class="container">
        <div class="error-message" id="error-message"></div>
        
        <div class="stats-grid">
            <div class="stat-card">
                <div class="stat-number" id="total-devices">-</div>
                <div class="stat-label">设备总数</div>
            </div>
            <div class="stat-card">
                <div class="stat-number" id="active-devices">-</div>
                <div class="stat-label">在线设备</div>
            </div>
            <div class="stat-card">
                <div class="stat-number" id="total-users">-</div>
                <div class="stat-label">用户总数</div>
            </div>
            <div class="stat-card">
                <div class="stat-number" id="active-connections">-</div>
                <div class="stat-label">活跃连接</div>
            </div>
        </div>
        
        <div class="dashboard-grid">
            <div class="card">
                <h3>设备管理</h3>
                <p>管理所有连接到NAT服务器的设备，查看设备状态和配置信息。</p>
                <button class="card-btn" onclick="window.location.href='/devices'">进入设备管理</button>
            </div>
            
            <div class="card">
                <h3>用户管理</h3>
                <p>管理系统用户，查看用户信息和权限设置。</p>
                <button class="card-btn" onclick="window.location.href='/users'">进入用户管理</button>
            </div>
            
            <div class="card">
                <h3>连接监控</h3>
                <p>实时监控NAT连接状态，查看网络流量和连接统计。</p>
                <button class="card-btn" onclick="window.location.href='/monitor'">进入连接监控</button>
            </div>
            
            <div class="card">
                <h3>系统设置</h3>
                <p>配置NAT服务器参数，调整网络设置和安全策略。</p>
                <button class="card-btn" onclick="window.location.href='/settings'">进入系统设置</button>
            </div>
        </div>
    </div>
    
    <script>
        // 检查用户登录状态
        function checkAuth() {
            const token = localStorage.getItem('jwt_token');
            const userInfo = localStorage.getItem('user_info');
            
            if (!token || !userInfo) {
                window.location.href = '/login';
                return;
            }
            
            try {
                const user = JSON.parse(userInfo);
                document.getElementById('username').textContent = user.username;
                
                // 加载统计数据
                loadStats();
            } catch (error) {
                console.error('解析用户信息失败:', error);
                logout();
            }
        }
        
        // 加载统计数据
        async function loadStats() {
            try {
                const token = localStorage.getItem('jwt_token');
                
                // 模拟统计数据（实际应该从API获取）
                document.getElementById('total-devices').textContent = Math.floor(Math.random() * 100) + 50;
                document.getElementById('active-devices').textContent = Math.floor(Math.random() * 50) + 20;
                document.getElementById('total-users').textContent = Math.floor(Math.random() * 200) + 100;
                document.getElementById('active-connections').textContent = Math.floor(Math.random() * 150) + 80;
            } catch (error) {
                console.error('加载统计数据失败:', error);
            }
        }
        
        // 退出登录
        function logout() {
            localStorage.removeItem('jwt_token');
            localStorage.removeItem('user_info');
            window.location.href = '/login';
        }
        
        // 页面加载时检查认证
        document.addEventListener('DOMContentLoaded', checkAuth);
    </script>
</body>
</html>"#;

    Html(html)
}

pub async fn devices_page() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>设备管理 - NAT Server</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }
        
        body {
            font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
            background: #f5f5f5;
            color: #333;
        }
        
        .header {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 1rem 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }
        
        .header h1 {
            font-size: 1.5rem;
        }
        
        .nav-links {
            display: flex;
            gap: 1rem;
        }
        
        .nav-links a {
            color: white;
            text-decoration: none;
            padding: 0.5rem 1rem;
            border-radius: 5px;
            transition: background 0.3s;
        }
        
        .nav-links a:hover, .nav-links a.active {
            background: rgba(255, 255, 255, 0.2);
        }
        
        .user-info {
            display: flex;
            align-items: center;
            gap: 1rem;
        }
        
        .logout-btn {
            background: rgba(255, 255, 255, 0.2);
            border: 1px solid rgba(255, 255, 255, 0.3);
            color: white;
            padding: 0.5rem 1rem;
            border-radius: 5px;
            cursor: pointer;
            transition: background 0.3s;
        }
        
        .logout-btn:hover {
            background: rgba(255, 255, 255, 0.3);
        }
        
        .container {
            max-width: 1200px;
            margin: 2rem auto;
            padding: 0 1rem;
        }
        
        .toolbar {
            background: white;
            padding: 1rem;
            border-radius: 10px;
            box-shadow: 0 2px 10px rgba(0, 0, 0, 0.1);
            margin-bottom: 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }
        
        .search-box {
            display: flex;
            gap: 0.5rem;
            align-items: center;
        }
        
        .search-box input {
            padding: 0.5rem;
            border: 2px solid #e1e5e9;
            border-radius: 5px;
            font-size: 1rem;
        }
        
        .search-box input:focus {
            outline: none;
            border-color: #667eea;
        }
        
        .btn {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            border: none;
            padding: 0.5rem 1rem;
            border-radius: 5px;
            cursor: pointer;
            transition: transform 0.2s;
        }
        
        .btn:hover {
            transform: translateY(-2px);
        }
        
        .btn-secondary {
            background: #6c757d;
        }
        
        .devices-grid {
            display: grid;
            gap: 1rem;
        }
        
        .device-card {
            background: white;
            padding: 1.5rem;
            border-radius: 10px;
            box-shadow: 0 2px 10px rgba(0, 0, 0, 0.1);
            display: flex;
            justify-content: space-between;
            align-items: center;
            transition: transform 0.2s;
        }
        
        .device-card:hover {
            transform: translateY(-2px);
        }
        
        .device-info h3 {
            color: #667eea;
            margin-bottom: 0.5rem;
        }
        
        .device-info p {
            color: #666;
            margin-bottom: 0.25rem;
        }
        
        .device-status {
            display: flex;
            align-items: center;
            gap: 0.5rem;
            margin-bottom: 0.5rem;
        }
        
        .status-dot {
            width: 10px;
            height: 10px;
            border-radius: 50%;
        }
        
        .status-online {
            background: #28a745;
        }
        
        .status-offline {
            background: #dc3545;
        }
        
        .device-actions {
            display: flex;
            gap: 0.5rem;
        }
        
        .btn-small {
            padding: 0.25rem 0.5rem;
            font-size: 0.8rem;
        }
        
        .btn-danger {
            background: #dc3545;
        }
        
        .modal {
            display: none;
            position: fixed;
            top: 0;
            left: 0;
            width: 100%;
            height: 100%;
            background: rgba(0, 0, 0, 0.5);
            justify-content: center;
            align-items: center;
        }
        
        .modal-content {
            background: white;
            padding: 2rem;
            border-radius: 10px;
            max-width: 400px;
            width: 90%;
        }
        
        .modal-header {
            margin-bottom: 1rem;
        }
        
        .modal-header h2 {
            color: #667eea;
        }
        
        .form-group {
            margin-bottom: 1rem;
        }
        
        .form-group label {
            display: block;
            margin-bottom: 0.5rem;
            color: #333;
            font-weight: 500;
        }
        
        .form-group input {
            width: 100%;
            padding: 0.75rem;
            border: 2px solid #e1e5e9;
            border-radius: 5px;
            font-size: 1rem;
        }
        
        .form-group input:focus {
            outline: none;
            border-color: #667eea;
        }
        
        .modal-footer {
            display: flex;
            gap: 1rem;
            justify-content: flex-end;
        }
        
        .error-message {
            background: #fee;
            color: #c33;
            padding: 0.75rem;
            border-radius: 5px;
            margin-bottom: 1rem;
            display: none;
        }
        
        .success-message {
            background: #efe;
            color: #3c3;
            padding: 0.75rem;
            border-radius: 5px;
            margin-bottom: 1rem;
            display: none;
        }
    </style>
</head>
<body>
    <div class="header">
        <h1>设备管理</h1>
        <div class="nav-links">
            <a href="/dashboard">控制台</a>
            <a href="/devices" class="active">设备管理</a>
            <a href="/users">用户管理</a>
            <a href="/monitor">连接监控</a>
        </div>
        <div class="user-info">
            <span id="username">加载中...</span>
            <button class="logout-btn" onclick="logout()">退出</button>
        </div>
    </div>
    
    <div class="container">
        <div class="error-message" id="error-message"></div>
        <div class="success-message" id="success-message"></div>
        
        <div class="toolbar">
            <div class="search-box">
                <input type="text" id="search-input" placeholder="搜索设备..." onkeyup="searchDevices()">
                <button class="btn btn-secondary" onclick="refreshDevices()">刷新</button>
            </div>
            <button class="btn" onclick="showAddDeviceModal()">添加设备</button>
        </div>
        
        <div class="devices-grid" id="devices-grid">
            <!-- 设备列表将通过JavaScript动态加载 -->
        </div>
    </div>
    
    <!-- 添加设备模态框 -->
    <div class="modal" id="add-device-modal">
        <div class="modal-content">
            <div class="modal-header">
                <h2>添加新设备</h2>
            </div>
            <form id="add-device-form">
                <div class="form-group">
                    <label for="device-id">设备ID</label>
                    <input type="text" id="device-id" name="device_id" required>
                </div>
                <div class="form-group">
                    <label for="device-name">设备名称</label>
                    <input type="text" id="device-name" name="device_name">
                </div>
                <div class="modal-footer">
                    <button type="button" class="btn btn-secondary" onclick="hideAddDeviceModal()">取消</button>
                    <button type="submit" class="btn">添加</button>
                </div>
            </form>
        </div>
    </div>
    
    <script>
        let devices = [];
        let currentUser = null;
        
        // 检查用户登录状态
        function checkAuth() {
            const token = localStorage.getItem('jwt_token');
            const userInfo = localStorage.getItem('user_info');
            
            if (!token || !userInfo) {
                window.location.href = '/login';
                return;
            }
            
            try {
                currentUser = JSON.parse(userInfo);
                document.getElementById('username').textContent = currentUser.username;
                
                // 加载设备列表
                loadDevices();
            } catch (error) {
                console.error('解析用户信息失败:', error);
                logout();
            }
        }
        
        // 加载设备列表
        async function loadDevices() {
            try {
                const token = localStorage.getItem('jwt_token');
                
                // 模拟设备数据（实际应该从API获取）
                devices = [
                    {
                        id: 'device001',
                        name: '办公室电脑',
                        status: 'online',
                        last_seen: '2024-01-15 10:30:00',
                        ip_address: '192.168.1.100',
                        user_id: currentUser.id
                    },
                    {
                        id: 'device002',
                        name: '家庭电脑',
                        status: 'offline',
                        last_seen: '2024-01-14 18:45:00',
                        ip_address: '192.168.1.101',
                        user_id: currentUser.id
                    },
                    {
                        id: 'device003',
                        name: '移动设备',
                        status: 'online',
                        last_seen: '2024-01-15 09:15:00',
                        ip_address: '192.168.1.102',
                        user_id: currentUser.id
                    }
                ];
                
                renderDevices();
            } catch (error) {
                console.error('加载设备列表失败:', error);
                showError('加载设备列表失败');
            }
        }
        
        // 渲染设备列表
        function renderDevices() {
            const grid = document.getElementById('devices-grid');
            grid.innerHTML = '';
            
            devices.forEach(device => {
                const card = document.createElement('div');
                card.className = 'device-card';
                card.innerHTML = `
                    <div class="device-info">
                        <h3>${device.name || device.id}</h3>
                        <div class="device-status">
                            <div class="status-dot status-${device.status}"></div>
                            <span>${device.status === 'online' ? '在线' : '离线'}</span>
                        </div>
                        <p>设备ID: ${device.id}</p>
                        <p>IP地址: ${device.ip_address}</p>
                        <p>最后在线: ${device.last_seen}</p>
                    </div>
                    <div class="device-actions">
                        <button class="btn btn-small" onclick="editDevice('${device.id}')">编辑</button>
                        <button class="btn btn-small btn-danger" onclick="removeDevice('${device.id}')">删除</button>
                    </div>
                `;
                grid.appendChild(card);
            });
        }
        
        // 搜索设备
        function searchDevices() {
            const searchTerm = document.getElementById('search-input').value.toLowerCase();
            const filteredDevices = devices.filter(device => 
                device.id.toLowerCase().includes(searchTerm) ||
                (device.name && device.name.toLowerCase().includes(searchTerm))
            );
            
            const grid = document.getElementById('devices-grid');
            grid.innerHTML = '';
            
            if (filteredDevices.length === 0) {
                grid.innerHTML = '<p style="text-align: center; color: #666;">没有找到匹配的设备</p>';
                return;
            }
            
            devices = filteredDevices;
            renderDevices();
            devices = devices.concat(filteredDevices.filter(d => !devices.includes(d)));
        }
        
        // 刷新设备列表
        function refreshDevices() {
            loadDevices();
            showSuccess('设备列表已刷新');
        }
        
        // 显示添加设备模态框
        function showAddDeviceModal() {
            document.getElementById('add-device-modal').style.display = 'flex';
        }
        
        // 隐藏添加设备模态框
        function hideAddDeviceModal() {
            document.getElementById('add-device-modal').style.display = 'none';
            document.getElementById('add-device-form').reset();
        }
        
        // 编辑设备
        function editDevice(deviceId) {
            const device = devices.find(d => d.id === deviceId);
            if (device) {
                // 填充表单
                document.getElementById('device-id').value = device.id;
                document.getElementById('device-name').value = device.name || '';
                
                // 显示模态框
                showAddDeviceModal();
            }
        }
        
        // 删除设备
        async function removeDevice(deviceId) {
            if (!confirm('确定要删除这个设备吗？')) {
                return;
            }
            
            try {
                const token = localStorage.getItem('jwt_token');
                
                // 模拟删除操作（实际应该调用API）
                devices = devices.filter(d => d.id !== deviceId);
                renderDevices();
                showSuccess('设备已删除');
            } catch (error) {
                console.error('删除设备失败:', error);
                showError('删除设备失败');
            }
        }
        
        // 退出登录
        function logout() {
            localStorage.removeItem('jwt_token');
            localStorage.removeItem('user_info');
            window.location.href = '/login';
        }
        
        // 显示错误消息
        function showError(message) {
            const errorDiv = document.getElementById('error-message');
            errorDiv.textContent = message;
            errorDiv.style.display = 'block';
            setTimeout(() => {
                errorDiv.style.display = 'none';
            }, 5000);
        }
        
        // 显示成功消息
        function showSuccess(message) {
            const successDiv = document.getElementById('success-message');
            successDiv.textContent = message;
            successDiv.style.display = 'block';
            setTimeout(() => {
                successDiv.style.display = 'none';
            }, 3000);
        }
        
        // 添加设备表单提交
        document.getElementById('add-device-form').addEventListener('submit', async function(e) {
            e.preventDefault();
            
            const deviceId = document.getElementById('device-id').value;
            const deviceName = document.getElementById('device-name').value;
            
            try {
                const token = localStorage.getItem('jwt_token');
                
                // 模拟添加设备操作（实际应该调用API）
                const newDevice = {
                    id: deviceId,
                    name: deviceName || deviceId,
                    status: 'offline',
                    last_seen: new Date().toLocaleString(),
                    ip_address: '未知',
                    user_id: currentUser.id
                };
                
                devices.unshift(newDevice);
                renderDevices();
                hideAddDeviceModal();
                showSuccess('设备已添加');
            } catch (error) {
                console.error('添加设备失败:', error);
                showError('添加设备失败');
            }
        });
        
        // 页面加载时检查认证
        document.addEventListener('DOMContentLoaded', checkAuth);
    </script>
</body>
</html>"#;

    Html(html)
}
