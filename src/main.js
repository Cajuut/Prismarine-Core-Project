// =============================================
// State & Core API
// =============================================
const { invoke } = window.__TAURI__.core;

let servers = [];
let systemStats = null;
let updateInterval = null;
let logRefreshIntervals = {};
let currentDetailServerId = null;
let detailLogInterval = null;
let managedPorts = [];
let isIpVisible = false;
let rawExternalIp = '取得中...';
let confirmResolve = null; // For custom confirm modal

let versionCache = {
    vanilla: null,
    paper: null,
    forge: null,
    mohist: null,
    banner: null
};

// =============================================
// App Initialization
// =============================================
document.addEventListener('DOMContentLoaded', async () => {
    console.log('[DEBUG] DOMContentLoaded - App initializing...');

    // 1. Initialize Event Listeners
    setupLifecycleListeners();
    initializeEventListeners();

    // 2. Initial Data Load
    try {
        await Promise.all([
            checkUPnPStatus(),
            updateExternalIP(),
            loadServers(),
            loadManagedPorts()
        ]);

        // 3. Start Background Tasks
        startMonitoring();

        console.log('[DEBUG] App initialization complete!');
    } catch (err) {
        console.error('[DEBUG] Early initialization error:', err);
    }
});

function setupLifecycleListeners() {
    window.addEventListener('beforeunload', () => {
        if (updateInterval) clearInterval(updateInterval);
        if (detailLogInterval) clearInterval(detailLogInterval);
        Object.values(logRefreshIntervals).forEach(interval => clearInterval(interval));
    });

    // Titlebar window controls
    const { getCurrentWindow } = window.__TAURI__.window;
    const appWindow = getCurrentWindow();

    // Titlebar dragging - entire titlebar except controls and IP
    document.querySelector('.titlebar')?.addEventListener('mousedown', (e) => {
        // Don't drag if clicking on controls, buttons, or external-ip
        if (e.target.closest('.titlebar-controls') ||
            e.target.closest('.action-btn-small') ||
            e.target.id === 'external-ip') {
            return;
        }
        if (e.buttons === 1) {
            appWindow.startDragging();
        }
    });

    document.getElementById('titlebar-minimize')?.addEventListener('click', () => {
        appWindow.minimize();
    });

    document.getElementById('titlebar-maximize')?.addEventListener('click', () => {
        appWindow.toggleMaximize();
    });

    document.getElementById('titlebar-close')?.addEventListener('click', () => {
        appWindow.close();
    });
}

function initializeEventListeners() {
    console.log('[DEBUG] registerEventListeners called');

    // Navigation
    document.querySelectorAll('.nav-item').forEach(item => {
        item.addEventListener('click', () => switchView(item.dataset.view));
    });

    // External IP click to copy
    document.getElementById('external-ip')?.addEventListener('click', (e) => {
        e.stopPropagation();
        e.preventDefault();
        const ip = rawExternalIp;
        if (ip && ip !== '取得中...' && ip !== '取得失敗') {
            navigator.clipboard.writeText(ip);
            showNotification('IPアドレスをコピーしました', 'success');
        }
    });

    // Sidebar UPnP status click (refresh)
    document.getElementById('upnp-indicator')?.parentElement?.addEventListener('click', checkUPnPStatus);

    // Create Server Modal Controls
    const createBtn = document.getElementById('create-server-btn');
    if (createBtn) {
        createBtn.onclick = () => {
            console.log('[DEBUG] Create Server button clicked');
            openCreateServerModal();
        };
    }

    document.getElementById('cancel-create-btn').onclick = closeCreateServerModal;
    document.getElementById('confirm-create-btn').onclick = createServer;

    document.querySelectorAll('.modal-close').forEach(btn => {
        btn.onclick = closeCreateServerModal;
    });

    // Close modal on background click
    document.getElementById('create-server-modal').addEventListener('click', (e) => {
        if (e.target.id === 'create-server-modal') closeCreateServerModal();
    });

    // Port Management Modal Controls
    document.getElementById('cancel-port-btn').onclick = closePortModal;
    document.getElementById('submit-port-btn').onclick = submitOpenPort;
    document.getElementById('port-modal').addEventListener('click', (e) => {
        if (e.target.id === 'port-modal') closePortModal();
    });

    // Server Detail View Controls
    document.getElementById('back-to-servers-btn').onclick = () => switchView('servers');
    document.getElementById('detail-start-btn').onclick = () => currentDetailServerId && startServer(currentDetailServerId);
    document.getElementById('detail-stop-btn').onclick = () => currentDetailServerId && stopServer(currentDetailServerId);
    document.getElementById('detail-delete-btn').onclick = () => currentDetailServerId && deleteServer(currentDetailServerId);
    document.getElementById('detail-delete-btn').onclick = () => currentDetailServerId && deleteServer(currentDetailServerId);
    document.getElementById('detail-clear-logs-btn').onclick = clearDetailLogs;
    document.getElementById('detail-copy-logs-btn').onclick = () => {
        const logsContent = document.getElementById('detail-logs-content')?.innerText;
        if (logsContent) {
            navigator.clipboard.writeText(logsContent).then(() => {
            }).catch(() => {
                showNotification('コピーに失敗しました', 'error');
            });
        }
    };

    document.getElementById('detail-restart-btn').onclick = () => currentDetailServerId && restartServer(currentDetailServerId);

    document.getElementById('detail-save-auto-restart-btn').onclick = saveAutoRestartSettings;
    document.getElementById('detail-auto-restart-toggle').onchange = saveAutoRestartSettings;
    document.getElementById('send-command-btn').onclick = sendConsoleCommand;
    document.getElementById('toggle-ip-btn').onclick = toggleIpVisibility;
    document.getElementById('save-motd-btn').onclick = saveMotd;
    document.getElementById('save-max-players-btn').onclick = saveMaxPlayers;
    document.getElementById('increase-players-btn').onclick = () => adjustMaxPlayers(1);
    document.getElementById('decrease-players-btn').onclick = () => adjustMaxPlayers(-1);
    document.getElementById('open-plugins-btn').onclick = openPluginsFolder;
    document.getElementById('detail-folder-btn').onclick = openServerFolder;

    const commandInput = document.getElementById('console-command-input');
    if (commandInput) {
        commandInput.onkeypress = (e) => {
            if (e.key === 'Enter') sendConsoleCommand();
        };
    }

    // Prismarine Bridge Controls
    document.getElementById('start-bridge-btn').onclick = startBridge;
    document.getElementById('stop-bridge-btn').onclick = stopBridge;
    document.getElementById('copy-bridge-address').onclick = copyBridgeAddress;
    document.getElementById('save-ngrok-token-btn')?.addEventListener('click', saveNgrokToken);

    // Plugins Store Controls
    document.getElementById('plugin-search-btn').onclick = searchPlugins;
    document.getElementById('plugin-search-input').onkeypress = (e) => {
        if (e.key === 'Enter') searchPlugins();
    };
    document.getElementById('plugin-source-select').onchange = () => {
        const container = document.getElementById('plugin-search-results');
        if (container) container.innerHTML = '';
        searchPlugins();
    };
    document.getElementById('install-starter-pack-btn').onclick = installStarterPack;
    document.getElementById('install-geyser-btn').onclick = installGeyser;
    document.getElementById('install-viaversion-btn').onclick = installViaVersion;

    // Tabs
    document.querySelectorAll('.detail-tab-btn').forEach(btn => {
        btn.onclick = () => switchTab(btn);
    });

    // Initialize custom selects
    setupCustomSelects();
    console.log('[DEBUG] registerEventListeners finished');
}

// =============================================
// UI Utility Functions
// =============================================

function setupCustomSelects() {
    document.querySelectorAll('.custom-select-wrapper').forEach(wrapper => {
        const select = wrapper.querySelector('.custom-select');
        const trigger = select.querySelector('.custom-select-trigger');
        const options = select.querySelectorAll('.custom-option');

        trigger.addEventListener('click', (e) => {
            select.classList.toggle('open');
            e.stopPropagation();
        });

        options.forEach(option => {
            option.addEventListener('click', (e) => {
                if (option.classList.contains('disabled')) return;

                select.querySelector('.custom-option.selected')?.classList.remove('selected');
                option.classList.add('selected');

                trigger.querySelector('span').textContent = option.textContent;
                select.classList.remove('open');

                // Trigger change event if needed or update hidden input
                e.stopPropagation();
            });
        });
    });

    window.addEventListener('click', () => {
        document.querySelectorAll('.custom-select').forEach(select => {
            select.classList.remove('open');
        });
    });
}

function switchView(viewName) {
    console.log('[DEBUG] Switching view to:', viewName);

    // Stop detail log refresh if leaving detail view
    if (currentDetailServerId && viewName !== 'server-detail') {
        if (detailLogInterval) {
            clearInterval(detailLogInterval);
            detailLogInterval = null;
        }
        currentDetailServerId = null;
    }

    // Hide all views, then show current
    document.querySelectorAll('.view').forEach(view => {
        view.classList.remove('active');
    });

    const targetView = (viewName === 'server-detail') ? 'server-detail-view' : `${viewName}-view`;
    const viewEl = document.getElementById(targetView);
    if (viewEl) {
        viewEl.classList.add('active');
        // Initial data sync for specific views
        if (viewName === 'ports') loadManagedPorts();
        if (viewName === 'servers' || viewName === 'dashboard') loadServers();
    } else {
        console.error('[DEBUG] View element not found:', targetView);
    }

    // Update Sidebar Navigation highlights
    document.querySelectorAll('.nav-item').forEach(item => {
        item.classList.toggle('active', item.dataset.view === viewName);
    });
}

function switchTab(btn) {
    const target = btn.dataset.target;

    // Update button states
    document.querySelectorAll('.detail-tab-btn').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');

    // Update content visibility
    document.querySelectorAll('.detail-tab-content').forEach(c => {
        if (c.id === `tab-content-${target}`) {
            c.classList.add('active');
            c.classList.remove('hidden');
        } else {
            c.classList.remove('active');
            c.classList.add('hidden');
        }
    });

    if (target === 'plugins') {
        const results = document.getElementById('plugin-search-results');
        if (results && (results.children.length === 0 || results.querySelector('.plugin-list-empty'))) {
            searchPlugins();
        }
    } else if (target === 'players') {
        refreshPlayerList();
    }
}

function showNotification(message, type = 'info') {
    console.log(`[NOTIFICATION][${type.toUpperCase()}] ${message}`);

    const existingContainer = document.getElementById('notification-container');
    const container = existingContainer || (() => {
        const c = document.createElement('div');
        c.id = 'notification-container';
        c.style.cssText = 'position: fixed; top: 50px; right: 20px; z-index: 99999; display: flex; flex-direction: column; gap: 10px; pointer-events: none;';
        document.body.appendChild(c);
        return c;
    })();

    const alert = document.createElement('div');
    alert.className = `notification notification-${type}`;
    alert.style.cssText = `
        padding: 12px 20px;
        min-width: 250px;
        background: rgba(15, 23, 42, 0.9);
        backdrop-filter: blur(10px);
        color: white;
        border-radius: 12px;
        border-left: 5px solid ${type === 'success' ? 'var(--success)' : type === 'error' ? 'var(--error)' : 'var(--primary)'};
        box-shadow: 0 10px 25px rgba(0,0,0,0.3);
        pointer-events: auto;
        animation: slideInNotification 0.3s cubic-bezier(0.4, 0, 0.2, 1);
        display: flex;
        align-items: center;
        gap: 12px;
        font-weight: 500;
        font-size: 0.9rem;
    `;

    const icon = type === 'success' ? '✅' : type === 'error' ? '❌' : 'ℹ️';
    alert.innerHTML = `<span>${icon}</span><span>${message}</span>`;

    container.appendChild(alert);

    setTimeout(() => {
        alert.style.opacity = '0';
        alert.style.transform = 'translateX(50px)';
        alert.style.transition = 'all 0.3s ease';
        setTimeout(() => alert.remove(), 300);
    }, 4000);
}

function showConfirmModal(message) {
    return new Promise((resolve) => {
        const modal = document.getElementById('confirm-modal');
        const msgEl = document.getElementById('confirm-modal-message');
        const okBtn = document.getElementById('confirm-modal-ok');
        const cancelBtn = document.getElementById('confirm-modal-cancel');
        const backdrop = modal.querySelector('.confirm-modal-backdrop');

        if (!modal) {
            resolve(confirm(message)); // Fallback to native
            return;
        }

        msgEl.textContent = message;
        modal.classList.add('active');

        confirmResolve = resolve;

        const cleanup = () => {
            modal.classList.remove('active');
            okBtn.removeEventListener('click', onOk);
            cancelBtn.removeEventListener('click', onCancel);
            backdrop.removeEventListener('click', onCancel);
        };

        const onOk = () => {
            cleanup();
            resolve(true);
        };

        const onCancel = () => {
            cleanup();
            resolve(false);
        };

        okBtn.addEventListener('click', onOk);
        cancelBtn.addEventListener('click', onCancel);
        backdrop.addEventListener('click', onCancel);
    });
}

function showEulaModal() {
    return new Promise((resolve) => {
        const modal = document.getElementById('eula-modal');
        const okBtn = document.getElementById('eula-modal-ok');
        const cancelBtn = document.getElementById('eula-modal-cancel');
        const backdrop = modal.querySelector('.eula-modal-backdrop');

        if (!modal) {
            resolve(confirm('Minecraft EULAに同意しますか？'));
            return;
        }

        modal.classList.add('active');

        const cleanup = () => {
            modal.classList.remove('active');
            okBtn.removeEventListener('click', onOk);
            cancelBtn.removeEventListener('click', onCancel);
            backdrop.removeEventListener('click', onCancel);
        };

        const onOk = () => {
            cleanup();
            resolve(true);
        };

        const onCancel = () => {
            cleanup();
            resolve(false);
        };

        okBtn.addEventListener('click', onOk);
        cancelBtn.addEventListener('click', onCancel);
        backdrop.addEventListener('click', onCancel);
    });
}

// Add notification style if missing
if (!document.getElementById('notif-style')) {
    const style = document.createElement('style');
    style.id = 'notif-style';
    style.textContent = `
        @keyframes slideInNotification {
            from { transform: translateX(100%); opacity: 0; }
            to { transform: translateX(0); opacity: 1; }
        }
    `;
    document.head.appendChild(style);
}

function escapeHtml(text) {
    if (!text) return '';
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

// =============================================
// Modal Logic
// =============================================

function openCreateServerModal() {
    console.log('[DEBUG] Executing openCreateServerModal');
    const modal = document.getElementById('create-server-modal');
    if (modal) {
        modal.classList.remove('hidden');
        // Force the app to apply the active class after the next frame for transition
        requestAnimationFrame(() => {
            modal.classList.add('active');
        });
        updateVersionList();
    } else {
        console.error('[DEBUG] Modal element not found: create-server-modal');
    }
}

function closeCreateServerModal() {
    const modal = document.getElementById('create-server-modal');
    if (modal) {
        modal.classList.remove('active');
        setTimeout(() => {
            modal.classList.add('hidden');
        }, 300);
    }
}

window.openPortModal = (slot) => {
    const modal = document.getElementById('port-modal');
    if (modal) {
        modal.dataset.slot = slot;
        modal.classList.remove('hidden');
        requestAnimationFrame(() => modal.classList.add('active'));
    }
};

function closePortModal() {
    const modal = document.getElementById('port-modal');
    if (modal) {
        modal.classList.remove('active');
        setTimeout(() => modal.classList.add('hidden'), 300);
    }
}

// =============================================
// Custom Select Logic
// =============================================

function setupCustomSelects() {
    document.querySelectorAll('.custom-select-wrapper').forEach(wrapper => {
        const select = wrapper.querySelector('.custom-select');
        const trigger = wrapper.querySelector('.custom-select-trigger');
        const customOptions = wrapper.querySelector('.custom-options');
        const hiddenInput = wrapper.querySelector('input[type="hidden"]');
        const triggerSpan = trigger.querySelector('span');

        if (!trigger || !customOptions) return;

        trigger.onclick = (e) => {
            e.stopPropagation();
            if (select.classList.contains('disabled')) return;

            // Close other selects
            document.querySelectorAll('.custom-select').forEach(s => {
                if (s !== select) s.classList.remove('open');
            });
            select.classList.toggle('open');
        };

        customOptions.onclick = (e) => {
            e.stopPropagation();
            const option = e.target.closest('.custom-option');
            if (!option || option.classList.contains('disabled')) return;

            // Update UI
            select.querySelector('.custom-option.selected')?.classList.remove('selected');
            option.classList.add('selected');
            triggerSpan.textContent = option.textContent;
            triggerSpan.classList.remove('placeholder');

            // Update value
            hiddenInput.value = option.dataset.value;
            select.classList.remove('open');

            // Dispatch Change
            hiddenInput.dispatchEvent(new Event('change', { bubbles: true }));
        };
    });

    // Close on body click
    document.addEventListener('click', () => {
        document.querySelectorAll('.custom-select').forEach(s => s.classList.remove('open'));
    });

    // Server type specific sync
    const typeInput = document.getElementById('server-type');
    if (typeInput) {
        typeInput.addEventListener('change', updateVersionList);
    }
}

// =============================================
// Server Management
// =============================================

async function loadServers() {
    try {
        servers = await invoke('get_servers') || [];
        renderServers();
        updateDashboard();
    } catch (err) {
        console.error('Failed to load servers:', err);
    }
}

function renderServers() {
    const serversList = document.getElementById('servers-list');
    const recentList = document.getElementById('recent-servers-list');

    if (!serversList) return;

    if (servers.length === 0) {
        serversList.innerHTML = '<div class="empty-state"><h3>サーバーが見つかりません</h3><p>「新規サーバー作成」から追加してください。</p></div>';
        if (recentList) recentList.innerHTML = '<div class="empty-state-text">サーバーなし</div>';
        return;
    }

    const html = servers.map(s => {
        // Handle Downloading status (comes as object like {Downloading: "message"})
        let statusText = s.status;
        let statusClass = '';
        let isDownloading = false;

        if (typeof s.status === 'object' && s.status.Downloading) {
            statusText = s.status.Downloading;
            statusClass = 'downloading';
            isDownloading = true;
        } else if (typeof s.status === 'string') {
            statusClass = s.status.toLowerCase();
            statusText = s.status;
        }

        const isRunning = s.status === 'Running';
        const isStartingOrDownloading = s.status === 'Starting' || isDownloading;

        return `
            <div class="server-card">
                <div class="server-card-header">
                    <h3 class="server-name">${escapeHtml(s.name)}</h3>
                    <span class="server-status ${statusClass}">${escapeHtml(statusText)}</span>
                </div>
                <div class="server-info">
                    <div class="server-info-item"><span>タイプ:</span><span>${s.server_type}</span></div>
                    <div class="server-info-item"><span>バージョン:</span><span>${s.version}</span></div>
                    <div class="server-info-item"><span>ポート:</span><span>${s.port}</span></div>
                </div>
                <div class="server-actions">
                    ${isRunning ?
                `<button class="btn btn-danger btn-sm" onclick="stopServer('${s.id}')">停止</button>` :
                isStartingOrDownloading ?
                    `<button class="btn btn-secondary btn-sm" disabled>準備中...</button>` :
                    `<button class="btn btn-primary btn-sm" onclick="startServer('${s.id}')">起動</button>`}
                    <button class="btn btn-secondary btn-sm" onclick="showServerDetail('${s.id}')">詳細</button>
                    <button class="btn btn-secondary btn-sm" onclick="deleteServer('${s.id}')">削除</button>
                </div>
            </div>
        `;
    }).join('');

    serversList.innerHTML = html;
    if (recentList) recentList.innerHTML = html; // Simple clone for now
}

async function startServer(id) {
    try {
        showNotification('サーバーを起動します...', 'info');
        await invoke('start_server', { serverId: id });
        await loadServers();
        if (currentDetailServerId === id) showServerDetail(id);
    } catch (err) {
        showNotification(`起動失敗: ${err}`, 'error');
    }
}

async function stopServer(id) {
    try {
        showNotification('サーバーを停止します...', 'info');
        await invoke('stop_server', { serverId: id });
        await loadServers();
        if (currentDetailServerId === id) showServerDetail(id);
    } catch (err) {
        showNotification(`停止失敗: ${err}`, 'error');
    }
}

async function deleteServer(id) {
    const server = servers.find(s => s.id === id);
    const name = server ? server.name : 'サーバー';
    const confirmed = await showConfirmModal(`「${name}」を完全に削除しますか？\nこの操作は元に戻せません。`);
    if (!confirmed) return;
    try {
        await invoke('delete_server', { serverId: id });
        showNotification('サーバーを削除しました', 'success');
        await loadServers();
        if (currentDetailServerId === id) switchView('servers');
    } catch (err) {
        showNotification(`削除失敗: ${err}`, 'error');
    }
}

// =============================================
// Server Details Functions
// =============================================

async function showServerDetail(id) {
    const server = servers.find(s => s.id === id);
    if (!server) return;

    currentDetailServerId = id;
    switchView('server-detail');

    // Populate Fields
    document.getElementById('detail-server-name').textContent = server.name;
    const statusEl = document.getElementById('detail-server-status');
    statusEl.textContent = server.status;
    statusEl.className = `server-status ${server.status.toLowerCase()}`;

    document.getElementById('detail-server-type').textContent = server.server_type;
    document.getElementById('detail-server-version').textContent = server.version;
    document.getElementById('detail-server-port').textContent = server.port;
    document.getElementById('detail-server-memory').textContent = server.max_memory;

    // Auto Restart Settings
    document.getElementById('detail-auto-restart-toggle').checked = server.auto_restart || false;
    const intervalHours = server.restart_interval ? Math.floor(server.restart_interval / 3600) : 24;
    document.getElementById('detail-restart-interval-input').value = intervalHours;

    const isRunning = server.status === 'Running';
    document.getElementById('detail-start-btn').style.display = isRunning ? 'none' : 'inline-flex';
    document.getElementById('detail-restart-btn').style.display = isRunning ? 'inline-flex' : 'none';
    document.getElementById('detail-stop-btn').style.display = isRunning ? 'inline-flex' : 'none';

    // Fetch MOTD & Max Players
    try {
        const motd = await invoke('get_motd', { serverId: id });
        document.getElementById('detail-server-motd-input').value = motd || '';

        const maxPlayers = await invoke('get_max_players', { serverId: id });
        document.getElementById('detail-server-max-players-input').value = maxPlayers;
        document.getElementById('detail-players').textContent = `0 / ${maxPlayers}`;

        // Check Geyser/ViaVersion status
        updatePresetButtons(id);
    } catch (e) { console.warn('Failed to fetch details:', e); }

    // Refresh Logs
    refreshDetailLogs();
    if (detailLogInterval) clearInterval(detailLogInterval);
    if (isRunning) {
        detailLogInterval = setInterval(refreshDetailLogs, 2000);
    }
}

async function updatePresetButtons(id) {
    const geyserArea = document.getElementById('geyser-status-area');
    const viaArea = document.getElementById('viaversion-status-area');
    if (!geyserArea || !viaArea) return;

    try {
        const geyserInstalled = await invoke('is_geyser_installed', { serverId: id });
        geyserArea.innerHTML = geyserInstalled ?
            `<button id="uninstall-geyser-btn" class="btn btn-danger btn-full" onclick="uninstallGeyser()">
                🗑️ クロスプレイを無効化
             </button>` :
            `<button id="install-geyser-btn" class="btn btn-secondary btn-full" onclick="installGeyser()">
                🔄 クロスプレイを有効化
             </button>`;

        const viaInstalled = await invoke('is_viaversion_installed', { serverId: id });
        viaArea.innerHTML = viaInstalled ?
            `<button id="uninstall-viaversion-btn" class="btn btn-danger btn-full" onclick="uninstallViaVersion()">
                🗑️ 互換性を無効化
             </button>` :
            `<button id="install-viaversion-btn" class="btn btn-secondary btn-full" onclick="installViaVersion()">
                🔄 互換性を有効化
             </button>`;
    } catch (e) {
        console.error('Failed to check preset status:', e);
    }
}

async function refreshDetailLogs() {
    if (!currentDetailServerId) return;
    const container = document.getElementById('detail-logs-content');
    if (!container) return;

    try {
        const logs = await invoke('get_server_logs', { serverId: currentDetailServerId, lines: 50 });
        if (logs.length === 0) {
            container.innerHTML = '<div class="logs-empty">ログなし</div>';
        } else {
            container.innerHTML = logs.map(line => {
                let cls = 'log-line';
                if (line.includes('ERROR')) cls += ' log-error';
                if (line.includes('WARN')) cls += ' log-warn';
                return `<div class="${cls}">${escapeHtml(line)}</div>`;
            }).join('');
            container.scrollTop = container.scrollHeight;
        }
    } catch (err) {
        container.innerHTML = `<div class="logs-error">ログ取得失敗: ${err}</div>`;
    }
}

async function sendConsoleCommand() {
    const input = document.getElementById('console-command-input');
    const cmd = input.value.trim();
    if (!cmd || !currentDetailServerId) return;

    try {
        await invoke('send_server_command', { serverId: currentDetailServerId, command: cmd });
        input.value = '';
        refreshDetailLogs();
    } catch (err) {
        showNotification(`送信失敗: ${err}`, 'error');
    }
}

function clearDetailLogs() {
    const el = document.getElementById('detail-logs-content');
    if (el) el.innerHTML = '';
}

async function saveMotd() {
    if (!currentDetailServerId) return;
    const motd = document.getElementById('detail-server-motd-input').value;
    try {
        await invoke('set_motd', { serverId: currentDetailServerId, motd });
        showNotification('MOTDを保存しました', 'success');
    } catch (e) { showNotification(e, 'error'); }
}

async function saveMaxPlayers() {
    if (!currentDetailServerId) return;
    const maxPlayers = parseInt(document.getElementById('detail-server-max-players-input').value);
    try {
        await invoke('set_max_players', { serverId: currentDetailServerId, maxPlayers });
        showNotification('最大人数を保存しました', 'success');
        document.getElementById('detail-players').textContent = `0 / ${maxPlayers}`;
    } catch (e) { showNotification(e, 'error'); }
}

function adjustMaxPlayers(delta) {
    const input = document.getElementById('detail-server-max-players-input');
    let val = parseInt(input.value) || 20;
    val = Math.max(1, val + delta);
    input.value = val;
}

async function openServerFolder() {
    if (!currentDetailServerId) return;
    try {
        await invoke('open_server_folder', { serverId: currentDetailServerId });
    } catch (e) { showNotification(e, 'error'); }
}

async function openPluginsFolder() {
    if (!currentDetailServerId) return;
    try {
        await invoke('open_plugins_folder', { serverId: currentDetailServerId });
    } catch (e) { showNotification(e, 'error'); }
}

// =============================================
// Versioning & Creation
// =============================================

async function updateVersionList() {
    const typeInp = document.getElementById('server-type');
    if (!typeInp) return;

    const type = typeInp.value;
    const wrapper = document.getElementById('wrapper-server-version');
    const select = wrapper.querySelector('.custom-select');
    const triggerSpan = select.querySelector('.custom-select-trigger span');
    const optionsCont = wrapper.querySelector('.custom-options');
    const hiddenInp = document.getElementById('server-version-select');

    select.classList.add('disabled');
    triggerSpan.textContent = '読み込み中...';
    optionsCont.innerHTML = '';

    try {
        let versions = [];
        if (type === 'vanilla') versions = await fetchVersions('vanilla');
        else if (type === 'paper') versions = await fetchVersions('paper');
        else if (type === 'forge') versions = await fetchVersions('forge');
        else if (type === 'mohist') versions = await fetchVersions('mohist');
        else if (type === 'banner') versions = await fetchVersions('banner');
        else if (type === 'purpur') versions = await fetchVersions('purpur');
        else if (type === 'spigot') versions = await fetchVersions('spigot');

        optionsCont.innerHTML = versions.map(v => `<span class="custom-option" data-value="${v}">${v}</span>`).join('');

        if (versions.length > 0) {
            select.classList.remove('disabled');
            const first = versions[0];
            triggerSpan.textContent = first;
            hiddenInp.value = first;
            optionsCont.querySelector('.custom-option').classList.add('selected');
        } else {
            triggerSpan.textContent = 'なし';
        }
    } catch (e) {
        triggerSpan.textContent = '取得失敗';
        console.error(e);
    }
}

async function fetchVersions(type) {
    if (versionCache[type]) return versionCache[type];
    const v = await invoke('fetch_versions', { serverType: type });
    versionCache[type] = v;
    return v;
}

async function createServer() {
    const name = document.getElementById('server-name').value.trim();
    const type = document.getElementById('server-type').value;
    const version = document.getElementById('server-version-select').value;
    const port = parseInt(document.getElementById('server-port').value);
    const memoryNum = document.getElementById('server-memory').value;
    const memory = memoryNum + 'G';  // Append 'G' to number
    const btn = document.getElementById('confirm-create-btn');

    if (!name || !version || !port) {
        showNotification('入力を確認してください', 'error');
        return;
    }

    // Show EULA confirmation
    const eulaAccepted = await showEulaModal();
    if (!eulaAccepted) {
        showNotification('EULAへの同意が必要です', 'info');
        return;
    }

    const oldText = btn.textContent;
    btn.disabled = true;
    btn.textContent = '作成中...';

    // Log container setup
    const logContainer = document.getElementById('create-log-container');
    const logControls = document.getElementById('log-controls');
    if (logContainer) {
        logContainer.style.display = 'none';
        logContainer.innerHTML = '';
    }
    if (logControls) {
        logControls.style.display = 'none';
    }

    let unlisten = null;

    try {
        // Setup build log listener
        if (window.__TAURI__ && window.__TAURI__.event) {
            unlisten = await window.__TAURI__.event.listen('build-log', (event) => {
                if (logContainer) {
                    logContainer.style.display = 'block';
                    logContainer.textContent += event.payload + '\n';
                    logContainer.scrollTop = logContainer.scrollHeight;
                }
                if (logControls) {
                    logControls.style.display = 'flex';
                }
            });
        }

        await invoke('create_server', { name, version, serverType: type, port, maxMemory: memory });
        showNotification('サーバーを作成しました', 'success');
        closeCreateServerModal();
        await loadServers();
    } catch (err) {
        showNotification(`作成失敗: ${err}`, 'error');
    } finally {
        btn.disabled = false;
        btn.textContent = oldText;
        if (unlisten) {
            unlisten();
        }
    }
}

// =============================================
// Settings & System
// =============================================

async function updateExternalIP() {
    try {
        const ip = await invoke('get_external_ip');
        const el = document.getElementById('external-ip');
        if (el) el.textContent = ip;
    } catch (e) { }
}

async function checkUPnPStatus() {
    try {
        const ok = await invoke('is_upnp_available');
        const ind = document.getElementById('upnp-indicator');
        const txt = document.getElementById('upnp-text');
        if (ind && txt) {
            ind.className = ok ? 'status-indicator online' : 'status-indicator offline';
            txt.textContent = ok ? 'UPnP利用可能' : 'UPnP利用不可';
        }
    } catch (e) { }
}

function startMonitoring() {
    updateSystemStats();
    updateInterval = setInterval(updateSystemStats, 3000);
}

async function updateSystemStats() {
    try {
        const stats = await invoke('get_system_stats');
        document.getElementById('cpu-usage').textContent = `${stats.cpu_usage.toFixed(1)}%`;
        document.getElementById('memory-usage').textContent = `${(stats.memory_used / 1024 / 1024).toFixed(0)} MB`;
        document.getElementById('cpu-percent').textContent = `${stats.cpu_usage.toFixed(1)}%`;
        document.getElementById('memory-percent').textContent = `${stats.memory_percent.toFixed(0)}%`;
    } catch (e) { }
}

async function updateExternalIP() {
    try {
        rawExternalIp = await invoke('get_external_ip');
        refreshIpDisplay();
    } catch (e) {
        rawExternalIp = '取得失敗';
        refreshIpDisplay();
    }
}

function toggleIpVisibility() {
    isIpVisible = !isIpVisible;
    refreshIpDisplay();
}

function refreshIpDisplay() {
    const el = document.getElementById('external-ip');
    const eyeIcon = document.getElementById('eye-icon');
    if (!el) return;

    if (isIpVisible) {
        el.textContent = rawExternalIp;
        el.classList.remove('hide-ip');
        if (eyeIcon) eyeIcon.innerHTML = '<path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"></path><circle cx="12" cy="12" r="3"></circle>';
    } else {
        el.textContent = '***.***.***.***';
        el.classList.add('hide-ip');
        if (eyeIcon) eyeIcon.innerHTML = '<path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"></path><line x1="1" y1="1" x2="23" y2="23"></line>';
    }
}

function updateDashboard() {
    document.getElementById('total-servers').textContent = servers.length;
    document.getElementById('running-servers').textContent = servers.filter(s => s.status === 'Running').length;
}

// =============================================
// Managed Ports
// =============================================

async function loadManagedPorts() {
    try {
        managedPorts = await invoke('get_managed_ports') || [];
        renderManagedPorts();
        const el = document.getElementById('port-count');
        if (el) el.textContent = `${managedPorts.length}/5`;
    } catch (e) {
        console.error(e);
    }
}

function renderManagedPorts() {
    const list = document.getElementById('managed-ports-list');
    if (!list) return;

    let html = '';
    for (let slot = 1; slot <= 5; slot++) {
        const p = managedPorts.find(x => x.slot === slot);
        if (p) {
            const isActive = p.active;
            const statusClass = isActive ? 'unlocked' : 'locked';
            const statusText = isActive ? '開放中' : '停止中';

            // Icons for actions
            const pauseIcon = `<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="4" width="4" height="16"></rect><rect x="14" y="4" width="4" height="16"></rect></svg>`;
            const playIcon = `<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg>`;
            const trashIcon = `<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"></polyline><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path></svg>`;

            // Use custom PNG assets
            const lockIconSrc = isActive ? 'icon-port-open.png' : 'icon-port-closed.png';

            html += `
                <div class="port-card ${statusClass}">
                    <div class="card-actions">
                        <button class="action-btn toggle" onclick="event.stopPropagation(); toggleManagedPort(${slot}, ${!isActive})" title="${isActive ? '一時停止' : '再開'}">
                            ${isActive ? pauseIcon : playIcon}
                        </button>
                        <button class="action-btn delete" onclick="event.stopPropagation(); deleteManagedPort(${slot})" title="削除">
                            ${trashIcon}
                        </button>
                    </div>
                    <div class="card-body">
                        <div class="port-lock-container">
                             <img src="${lockIconSrc}" class="port-icon-large" alt="${statusText}">
                        </div>
                        <span class="port-name">${escapeHtml(p.name)}</span>
                        <div class="port-number-display">${p.port}</div>
                    </div>
                    <div class="port-status-bar" onclick="toggleManagedPort(${slot}, ${!isActive})">
                        ${statusText}
                    </div>
                </div>
            `;
        } else {
            html += `
                <div class="port-card empty clickable" onclick="openPortModal(${slot})">
                    <div class="card-body">
                        <div class="empty-slot-content">
                            <span class="plus-icon">+</span>
                            <span class="empty-text">空きスロット</span>
                            <span class="slot-label">Slot ${slot}</span>
                        </div>
                    </div>
                </div>
            `;
        }
    }
    list.innerHTML = html;
}

async function submitOpenPort() {
    const modal = document.getElementById('port-modal');
    const port = parseInt(document.getElementById('port-number-input').value);
    const protocol = document.querySelector('input[name="protocol"]:checked').value;
    const name = document.getElementById('port-name-input').value || 'Server';
    const slot = parseInt(modal.dataset.slot);

    try {
        await invoke('open_managed_port', { port, protocol, name, slot });
        closePortModal();
        await loadManagedPorts();
    } catch (err) {
        showNotification(`エラー: ${err}`, 'error');
    }
}

window.toggleManagedPort = async (slot, active) => {
    try {
        await invoke('set_managed_port_active', { slot, active });
        await loadManagedPorts();
    } catch (e) { showNotification(e, 'error'); }
};

window.deleteManagedPort = async (slot) => {
    const confirmed = await showConfirmModal('このポート設定を削除しますか？\nポート開放は解除されます。');
    if (!confirmed) return;
    try {
        await invoke('delete_managed_port', { slot });
        await loadManagedPorts();
        showNotification('ポート設定を削除しました', 'success');
    } catch (e) { showNotification(e, 'error'); }
};

// =============================================
// Plugins & Bridge (Simplified for now)
// =============================================


async function searchPlugins() {
    const q = document.getElementById('plugin-search-input').value;
    const s = document.getElementById('plugin-source-select').value;
    const res = document.getElementById('plugin-search-results');
    if (!res) return;

    res.innerHTML = '<div class="loading-spinner-container"><div class="loading-spinner"></div><p>' + (q ? 'プラグインを検索中...' : '人気のプラグインを取得中...') + '</p></div>';

    try {
        const list = await invoke('search_plugins', { serverId: currentDetailServerId, query: q, source: s });
        if (list.length === 0) {
            res.innerHTML = '<div class="plugin-list-empty">一致するプラグインが見つかりませんでした。</div>';
            return;
        }

        const title = q ? '' : `<h3 class="plugin-section-title">${s} の人気プラグイン</h3>`;

        const pluginCards = await Promise.all(list.map(async p => {
            const icon = p.icon_url || 'https://cdn.modrinth.com/placeholder.svg';
            const author = p.author ? `by ${p.author}` : '';
            const downloads = p.downloads ? `• ${p.downloads.toLocaleString()} downloads` : '';

            // Check if installed
            let isInstalled = false;
            try {
                isInstalled = await invoke('is_plugin_installed', { serverId: currentDetailServerId, pluginId: p.id, source: s });
            } catch (e) { /* ignore */ }

            const actionBtn = isInstalled
                ? `<button class="btn btn-danger btn-sm" id="install-btn-${p.id}" onclick="uninstallPlugin('${p.id}', '${escapeHtml(p.name)}', '${s}')">無効化</button>`
                : `<button class="btn btn-primary btn-sm" id="install-btn-${p.id}" onclick="installPlugin('${p.id}', '${escapeHtml(p.name)}', '${s}')">追加</button>`;

            return `
                <div class="plugin-card ${isInstalled ? 'installed' : ''}">
                    <img src="${icon}" class="plugin-icon" alt="${p.name}" onerror="this.src='https://cdn.modrinth.com/placeholder.svg'">
                    <div class="plugin-info">
                        <div class="plugin-name">
                            ${escapeHtml(p.name)}
                            <span class="plugin-source-badge">${s}</span>
                        </div>
                        <div class="plugin-desc">${escapeHtml(p.description || '説明はありません。')}</div>
                        <div class="plugin-meta">
                            ${author} ${downloads} 
                            • <a href="${p.external_url}" target="_blank" class="plugin-link">🌐 Webで見る</a>
                        </div>
                    </div>
                    <div class="plugin-actions">
                        ${actionBtn}
                    </div>
                </div>
            `;
        }));
        res.innerHTML = title + pluginCards.join('');
    } catch (e) {
        res.innerHTML = `<div class="plugin-list-empty error">取得失敗: ${e}</div>`;
    }
}

async function uninstallPlugin(id, name, source) {
    const btn = document.getElementById(`install-btn-${id}`);
    if (!btn) return;

    const confirmed = await showConfirmModal(`「${name}」を削除しますか？\nこの操作は元に戻せません。`);
    if (!confirmed) return;

    btn.disabled = true;
    btn.textContent = '削除中...';

    try {
        await invoke('uninstall_plugin', { serverId: currentDetailServerId, pluginId: id, source: source });
        showNotification(`${name} を削除しました`, 'success');
        btn.textContent = '削除完了';
        // Refresh status if simple search results
        setTimeout(() => searchPlugins(), 1500);
    } catch (e) {
        showNotification(`削除に失敗しました: ${e}`, 'error');
        btn.disabled = false;
        btn.textContent = '無効化';
    }
}

async function installPlugin(id, name, source) {
    const btn = document.getElementById(`install-btn-${id}`);
    const oldText = btn ? btn.textContent : '追加';
    if (btn) {
        btn.disabled = true;
        btn.textContent = '導入中...';
    }

    try {
        showNotification(`${name} を導入しています...`, 'info');
        if (source === 'Spigot') {
            await invoke('install_spigot_plugin', { serverId: currentDetailServerId, resourceId: id });
        } else {
            await invoke('install_modrinth_plugin', { serverId: currentDetailServerId, projectId: id });
        }
        showNotification(`${name} の導入が完了しました`, 'success');

        if (btn) {
            btn.disabled = false;
            btn.textContent = '無効化';
            btn.className = 'btn btn-danger btn-sm';
            btn.onclick = () => uninstallPlugin(id, name, source);
            // Add installed class to card
            btn.closest('.plugin-card')?.classList.add('installed');
        }
    } catch (e) {
        showNotification(`${name} の導入に失敗しました: ${e}`, 'error');
        if (btn) {
            btn.disabled = false;
            btn.textContent = oldText;
        }
    }
}

async function installStarterPack() {
    if (!currentDetailServerId) return;
    const btn = document.getElementById('install-starter-pack-btn');
    if (!btn) return;

    const oldText = btn.textContent;
    btn.disabled = true;
    btn.textContent = '導入中...';

    const plugins = [
        { id: 'luckperms', name: 'LuckPerms' },
        { id: 'essentialsx', name: 'EssentialsX' },
        { id: 'worldedit', name: 'WorldEdit' },
        { id: 'vault', name: 'Vault' },
        { id: 'coreprotect', name: 'CoreProtect' },
        { id: 'quickshop-hikari', name: 'QuickShop-Hikari' }
    ];

    try {
        for (const p of plugins) {
            showNotification(`${p.name} を導入中...`, 'info');
            await invoke('install_modrinth_plugin', { serverId: currentDetailServerId, projectId: p.id });
        }
        showNotification('スターターパックの導入が完了しました', 'success');
        btn.textContent = '導入完了';
    } catch (e) {
        showNotification(`エラーが発生しました: ${e}`, 'error');
        btn.disabled = false;
        btn.textContent = oldText;
    }
}

async function startBridge() {
    showNotification('Bridgeを起動中...', 'info');
    try {
        await invoke('start_bridge', { port: 25565 });
        document.getElementById('bridge-status-text').textContent = '起動中';
    } catch (e) { showNotification(e, 'error'); }
}

async function stopBridge() {
    await invoke('stop_bridge');
    document.getElementById('bridge-status-text').textContent = '停止中';
}

function copyBridgeAddress() {
    const addr = document.getElementById('bridge-address-text').textContent;
    navigator.clipboard.writeText(addr);
    showNotification('コピーしました', 'success');
}

async function installGeyser() {
    if (!currentDetailServerId) return;
    const btn = document.getElementById('install-geyser-btn');
    const oldText = btn.textContent;
    btn.disabled = true;
    btn.textContent = '導入中...';
    try {
        await invoke('install_geyser_support', { serverId: currentDetailServerId });
        showNotification('Geyserを追加しました', 'success');
        updatePresetButtons(currentDetailServerId);
    } catch (e) {
        showNotification(e, 'error');
        btn.disabled = false;
        btn.textContent = oldText;
    }
}

async function installViaVersion() {
    if (!currentDetailServerId) return;
    const btn = document.getElementById('install-viaversion-btn');
    const oldText = btn.textContent;
    btn.disabled = true;
    btn.textContent = '導入中...';
    try {
        await invoke('install_viaversion_support', { serverId: currentDetailServerId });
        showNotification('ViaVersionを追加しました', 'success');
        updatePresetButtons(currentDetailServerId);
    } catch (e) {
        showNotification(e, 'error');
        btn.disabled = false;
        btn.textContent = oldText;
    }
}

async function uninstallGeyser() {
    if (!currentDetailServerId) return;
    const btn = document.getElementById('uninstall-geyser-btn');
    const oldText = btn.textContent;
    btn.disabled = true;
    btn.textContent = '削除中...';
    try {
        await invoke('uninstall_geyser_support', { serverId: currentDetailServerId });
        showNotification('Geyserを削除しました', 'success');
        updatePresetButtons(currentDetailServerId);
    } catch (e) {
        showNotification(e, 'error');
        btn.disabled = false;
        btn.textContent = oldText;
    }
}

async function uninstallViaVersion() {
    if (!currentDetailServerId) return;
    const btn = document.getElementById('uninstall-viaversion-btn');
    const oldText = btn.textContent;
    btn.disabled = true;
    btn.textContent = '削除中...';
    try {
        await invoke('uninstall_viaversion_support', { serverId: currentDetailServerId });
        showNotification('ViaVersionを削除しました', 'success');
        updatePresetButtons(currentDetailServerId);
    } catch (e) {
        showNotification(e, 'error');
        btn.disabled = false;
        btn.textContent = oldText;
    }
}


async function restartServer(id) {

    try {
        showNotification('再起動を開始します...', 'info');
        await invoke('restart_server', { serverId: id });
        showNotification('再起動コマンドを送信しました', 'success');
        // Refresh status after a delay
        setTimeout(loadServers, 3000);
    } catch (e) {
        showNotification(`再起動失敗: ${e}`, 'error');
    }
}

async function saveAutoRestartSettings() {
    if (!currentDetailServerId) return;
    const enabled = document.getElementById('detail-auto-restart-toggle').checked;
    const interval = parseInt(document.getElementById('detail-restart-interval-input').value, 10) * 3600; // Hours to seconds

    try {
        await invoke('set_auto_restart', {
            serverId: currentDetailServerId,
            enabled: enabled,
            interval: interval
        });
        showNotification('自動再起動設定を保存しました', 'success');
    } catch (e) {
        showNotification(`設定保存失敗: ${e}`, 'error');
    }
}

async function refreshPlayerList() {
    if (!currentDetailServerId) return;
    const listContainer = document.getElementById('detail-players-list');
    try {
        const players = await invoke('get_online_players', { serverId: currentDetailServerId });
        if (players && players.length > 0) {
            listContainer.innerHTML = players.map(p => `
                <div class="player-item">
                    <img src="https://mc-heads.net/avatar/${p}/32" alt="${p}">
                    <span>${p}</span>
                </div>
            `).join('');
            document.getElementById('detail-players').textContent = `${players.length} / ${document.getElementById('detail-server-max-players-input').value}`;
        } else {
            listContainer.innerHTML = '<div class="empty-state">オフラインまたはプレイヤーはいません</div>';
            document.getElementById('detail-players').textContent = `0 / ${document.getElementById('detail-server-max-players-input').value}`;
        }
    } catch (e) {
        // console.warn('Failed to fetch player list:', e);
        listContainer.innerHTML = '<div class="empty-state">取得失敗 (サーバー停止中?)</div>';
    }
}

console.log('[DEBUG] main.js loaded');

function copyBuildLog() {
    const el = document.getElementById('create-log-container');
    if (!el) return;
    navigator.clipboard.writeText(el.textContent)
        .then(() => showNotification('ログをコピーしました', 'success'))
        .catch(err => showNotification(`コピー失敗: ${err}`, 'error'));
}
window.copyBuildLog = copyBuildLog;
