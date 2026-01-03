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
let detailUptimeInterval = null;
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
    // Toggle onchange is handled via HTML onchange="toggleAutoRestartSettings()"
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
    document.getElementById('store-type-select').onchange = () => {
        updateStoreUIForType();
    };
    document.getElementById('plugin-refresh-btn').onclick = searchPlugins;
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

    // Update UI based on server type (Vanilla/Mod/Plugin)
    updateUIForServerType(server.server_type);

    // Auto Restart Settings
    const autoRestartToggle = document.getElementById('detail-auto-restart-toggle');
    autoRestartToggle.checked = server.auto_restart || false;

    // Set Mode
    const restartType = server.restart_type || 'Interval';
    const modeRadio = document.querySelector(`input[name="restartMode"][value="${restartType}"]`);
    if (modeRadio) modeRadio.checked = true;

    // Set Interval (H:M:S)
    const interval = server.restart_interval || 86400;
    const h = Math.floor(interval / 3600);
    const m = Math.floor((interval % 3600) / 60);
    const s = interval % 60;

    document.getElementById('restart-interval-h').value = h;
    document.getElementById('restart-interval-m').value = m;
    document.getElementById('restart-interval-s').value = s;

    // Set Schedule
    if (server.restart_schedule) {
        document.getElementById('restart-schedule-time').value = server.restart_schedule;
    }
    if (server.time_zone) {
        document.getElementById('restart-timezone').value = server.time_zone;
    } else {
        document.getElementById('restart-timezone').value = 'Asia/Tokyo';
    }

    toggleAutoRestartSettings();
    toggleAutoRestartMode();

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
    if (detailUptimeInterval) clearInterval(detailUptimeInterval);

    if (isRunning) {
        detailLogInterval = setInterval(refreshDetailLogs, 2000);

        // Uptime Updater
        const updateUptime = () => {
            if (!server.last_start_time) return;
            const now = Math.floor(Date.now() / 1000);
            const uptimeSeconds = now - server.last_start_time;

            if (uptimeSeconds < 0) {
                document.getElementById('detail-uptime').textContent = '起動中...';
                return;
            }

            const h = Math.floor(uptimeSeconds / 3600);
            const m = Math.floor((uptimeSeconds % 3600) / 60);
            const s = uptimeSeconds % 60;

            document.getElementById('detail-uptime').textContent = `${h}時間 ${m}分 ${s}秒`;
        };

        updateUptime();
        detailUptimeInterval = setInterval(updateUptime, 1000);
    } else {
        document.getElementById('detail-uptime').textContent = '-';
    }
}

// Helper to determine server type category
function getServerCategory(serverType) {
    const modServers = ['Fabric', 'Forge'];
    const pluginServers = ['Paper', 'Spigot', 'Purpur'];
    const hybridServers = ['Mohist', 'Taiyitist', 'Banner']; // Supports both plugins and mods
    const proxyServers = ['Velocity', 'Waterfall', 'BungeeCord'];

    if (serverType === 'Vanilla') return 'vanilla';
    if (proxyServers.includes(serverType)) return 'proxy';
    if (hybridServers.includes(serverType)) return 'hybrid';
    if (modServers.includes(serverType)) return 'mod';
    if (pluginServers.includes(serverType)) return 'plugin';
    return 'plugin'; // Default to plugin
}

// Store the current server type globally for reference
let currentServerType = null;

// Update UI based on server type
function updateUIForServerType(serverType) {
    currentServerType = serverType;
    const category = getServerCategory(serverType);

    // Get UI elements
    const pluginTabBtn = document.querySelector('.detail-tab-btn[data-target="plugins"]');
    const geyserArea = document.getElementById('geyser-status-area');
    const viaArea = document.getElementById('viaversion-status-area');
    const openPluginsBtn = document.getElementById('open-plugins-btn');
    const storeTypeSelect = document.getElementById('store-type-select');
    const starterPackBanner = document.querySelector('.starter-pack-banner');

    // Plugin Store Tab - hide for Vanilla
    if (pluginTabBtn) {
        pluginTabBtn.style.display = category === 'vanilla' ? 'none' : 'inline-flex';
        // Update label based on category
        if (category === 'mod') {
            pluginTabBtn.textContent = 'Modストア';
        } else if (category === 'hybrid') {
            pluginTabBtn.textContent = 'Plugin/Modストア';
        } else if (category === 'plugin' || category === 'proxy') {
            pluginTabBtn.textContent = 'プラグインストア';
        }
    }

    // Show store type selector for hybrid servers (Mohist)
    if (storeTypeSelect) {
        storeTypeSelect.style.display = category === 'hybrid' ? 'inline-flex' : 'none';
    }

    // Geyser/ViaVersion areas - show for plugin and hybrid servers and proxy
    const showGeyser = category === 'plugin' || category === 'hybrid' || category === 'proxy';
    if (geyserArea) geyserArea.style.display = showGeyser ? 'block' : 'none';
    if (viaArea) viaArea.style.display = showGeyser ? 'block' : 'none';

    // Starter Pack - only for plugin and hybrid servers (not proxy)
    if (starterPackBanner) {
        starterPackBanner.style.display = (category === 'plugin' || category === 'hybrid') ? 'flex' : 'none';
    }

    // Open Plugins/Mods folder button
    if (openPluginsBtn) {
        openPluginsBtn.style.display = category === 'vanilla' ? 'none' : 'inline-flex';
        if (category === 'mod') {
            openPluginsBtn.textContent = '📁 Modsフォルダを開く';
        } else if (category === 'hybrid') {
            openPluginsBtn.textContent = '📁 Plugins/Modsフォルダを開く';
        } else {
            openPluginsBtn.textContent = '📁 Pluginsフォルダを開く';
        }
    }

    // Update store UI elements
    updateStoreUIForType();
}

// Update store-specific UI elements based on store type selection
function updateStoreUIForType() {
    const category = getServerCategory(currentServerType);
    const storeTypeSelect = document.getElementById('store-type-select');
    const searchInput = document.getElementById('plugin-search-input');
    const sourceSelect = document.getElementById('plugin-source-select');
    const spigotOption = sourceSelect?.querySelector('option[value="Spigot"]');

    // Determine effective store type
    let storeType = 'plugin'; // default
    if (category === 'mod') {
        storeType = 'mod';
    } else if (category === 'hybrid' && storeTypeSelect) {
        storeType = storeTypeSelect.value;
    }

    // Update placeholder
    if (searchInput) {
        searchInput.placeholder = storeType === 'mod'
            ? 'Modを検索... (例: JEI, Sodium)'
            : 'プラグインを検索... (例: WorldEdit, Essentials)';
    }

    // Spigot option only available for plugins, not mods
    if (spigotOption) {
        spigotOption.style.display = storeType === 'mod' ? 'none' : 'block';
        // If currently on Spigot and switching to mod mode, force switch to Modrinth
        if (storeType === 'mod' && sourceSelect.value === 'Spigot') {
            sourceSelect.value = 'Modrinth';
        }
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
        else if (type === 'spigot') versions = await fetchVersions('spigot');
        else if (type === 'forge') versions = await fetchVersions('forge');
        else if (type === 'fabric') versions = await fetchVersions('fabric');
        else if (type === 'mohist') versions = await fetchVersions('mohist');
        else if (type === 'taiyitist') versions = await fetchVersions('taiyitist');
        else if (type === 'purpur') versions = await fetchVersions('purpur');
        else if (type === 'banner') versions = await fetchVersions('banner');
        else if (type === 'velocity') versions = await fetchVersions('velocity');
        else if (type === 'waterfall') versions = await fetchVersions('waterfall');
        else if (type === 'bungeecord') versions = await fetchVersions('bungeecord');
        else {
            console.warn('[updateVersionList] Unknown server type:', type);
            versions = await fetchVersions('vanilla'); // Fallback
        }

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

    try {
        await invoke('create_server', { name, version, serverType: type, port, maxMemory: memory });
        showNotification('サーバーを作成しました', 'success');
        closeCreateServerModal();
        await loadServers();
    } catch (err) {
        showNotification(`作成失敗: ${err}`, 'error');
    } finally {
        btn.disabled = false;
        btn.textContent = oldText;
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
        await invoke('uninstall_plugin', { serverId: currentDetailServerId, pluginName: name });
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
            await invoke('install_spigot_plugin', { serverId: currentDetailServerId, resourceId: id, pluginName: name });
        } else {
            await invoke('install_modrinth_plugin', { serverId: currentDetailServerId, projectId: id, pluginName: name });
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
    const mode = document.querySelector('input[name="restartMode"]:checked').value;

    // Get Interval (convert to seconds)
    const h = parseInt(document.getElementById('restart-interval-h').value) || 0;
    const m = parseInt(document.getElementById('restart-interval-m').value) || 0;
    const s = parseInt(document.getElementById('restart-interval-s').value) || 0;
    const interval = (h * 3600) + (m * 60) + s;

    // Get Schedule
    const scheduleTime = document.getElementById('restart-schedule-time').value;
    const timezone = document.getElementById('restart-timezone').value;

    let schedule = null;
    if (mode === 'Schedule' && scheduleTime) {
        schedule = scheduleTime;
    }

    try {
        await invoke('set_auto_restart', {
            serverId: currentDetailServerId,
            enabled: enabled,
            restartType: mode,
            interval: interval,
            schedule: schedule,
            timeZone: timezone
        });
        showNotification('自動再起動設定を保存しました', 'success');
    } catch (e) {
        showNotification(`設定保存失敗: ${e}`, 'error');
    }
}

function toggleAutoRestartSettings() {
    const enabled = document.getElementById('detail-auto-restart-toggle').checked;
    const options = document.getElementById('auto-restart-options');

    options.style.opacity = enabled ? '1' : '0.5';

    // Toggle all inputs inside
    const inputs = options.querySelectorAll('input, select, button');
    inputs.forEach(input => {
        input.disabled = !enabled;
    });

    if (enabled) {
        toggleAutoRestartMode();
    }
}

function toggleAutoRestartMode() {
    const enabled = document.getElementById('detail-auto-restart-toggle').checked;
    const mode = document.querySelector('input[name="restartMode"]:checked').value;

    const intervalSettings = document.getElementById('restart-interval-settings');
    const scheduleSettings = document.getElementById('restart-schedule-settings');

    if (mode === 'Interval') {
        intervalSettings.classList.remove('hidden');
        scheduleSettings.classList.add('hidden');
    } else {
        intervalSettings.classList.add('hidden');
        scheduleSettings.classList.remove('hidden');
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

// =============================================
// Visual Network Editor v2 (ProxyNodes)
// =============================================

let networkNodes = {}; // { serverId: { x, y } }
let networkConnections = []; // [{ from: serverId, to: serverId }]
let isConnectMode = false;
let connectFromNode = null;
let draggedNode = null;
let dragOffset = { x: 0, y: 0 };
// Canvas panning
let isPanning = false;
let panStart = { x: 0, y: 0 };
let canvasOffset = { x: 0, y: 0 };

// Hook into Network Tab Click
document.addEventListener('DOMContentLoaded', () => {
    const networkBtn = document.querySelector('.nav-item[data-view="network"]');
    if (networkBtn) {
        networkBtn.addEventListener('click', () => {
            console.log('[Network] Switching to Network View');
            initNetworkEditor();
        });
    }

    // Setup canvas event listeners
    const canvas = document.getElementById('network-canvas');
    if (canvas) {
        canvas.addEventListener('mousedown', onCanvasMouseDown);
        canvas.addEventListener('mousemove', onCanvasMouseMove);
        canvas.addEventListener('mouseup', onCanvasMouseUp);
        canvas.addEventListener('mouseleave', onCanvasMouseUp);
    }
});

async function initNetworkEditor() {
    await loadServers();
    loadNetworkLayout();
    renderNetworkPalette();
    renderNetworkCanvas();
    renderConnectionList();
}

function loadNetworkLayout() {
    try {
        const saved = localStorage.getItem('networkLayout');
        if (saved) {
            const data = JSON.parse(saved);
            networkNodes = data.nodes || {};
            networkConnections = data.connections || [];
        }
    } catch (e) {
        console.warn('Failed to load network layout:', e);
    }

    // Initialize positions for servers not yet placed
    const canvasRect = document.getElementById('network-canvas')?.getBoundingClientRect();
    const centerX = (canvasRect?.width || 800) / 2;
    const centerY = (canvasRect?.height || 600) / 2;

    servers.forEach((s, i) => {
        if (!networkNodes[s.id]) {
            const angle = (i / servers.length) * 2 * Math.PI;
            const radius = 150;
            networkNodes[s.id] = {
                x: centerX + Math.cos(angle) * radius,
                y: centerY + Math.sin(angle) * radius
            };
        }
    });

    // Clean up orphaned connections (servers that no longer exist)
    const serverIds = servers.map(s => s.id);
    networkConnections = networkConnections.filter(conn =>
        serverIds.includes(conn.from) && serverIds.includes(conn.to)
    );

    // Clean up orphaned node positions
    Object.keys(networkNodes).forEach(id => {
        if (!serverIds.includes(id)) {
            delete networkNodes[id];
        }
    });
}

function saveNetworkLayout() {
    try {
        localStorage.setItem('networkLayout', JSON.stringify({
            nodes: networkNodes,
            connections: networkConnections
        }));
        showNotification('レイアウトを保存しました', 'success');
    } catch (e) {
        console.error('Failed to save network layout:', e);
        showNotification('保存に失敗しました', 'error');
    }
}

let isNetworkFullscreen = false;

function toggleNetworkFullscreen() {
    const wrapper = document.getElementById('network-canvas-wrapper');
    const btn = document.getElementById('network-fullscreen-btn');
    const editorContainer = wrapper?.parentElement;

    if (!wrapper || !btn) return;

    isNetworkFullscreen = !isNetworkFullscreen;

    if (isNetworkFullscreen) {
        // Enter fullscreen
        wrapper.style.cssText = `
            position: fixed !important;
            top: 32px;
            left: 0;
            right: 0;
            bottom: 0;
            width: 100vw;
            height: calc(100vh - 32px);
            z-index: 1000;
            background: rgba(10,15,25,0.95);
            border-radius: 0;
            border: none;
        `;
        btn.innerHTML = `
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="4 14 10 14 10 20"></polyline>
                <polyline points="20 10 14 10 14 4"></polyline>
                <line x1="14" y1="10" x2="21" y2="3"></line>
                <line x1="3" y1="21" x2="10" y2="14"></line>
            </svg>
            最小化
        `;
        document.body.style.overflow = 'hidden';
    } else {
        // Exit fullscreen
        wrapper.style.cssText = `
            flex: 1;
            background: rgba(10,15,25,0.8);
            border-radius: 8px;
            position: relative;
            border: 1px solid rgba(255,255,255,0.1);
            overflow: hidden;
        `;
        btn.innerHTML = `
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="15 3 21 3 21 9"></polyline>
                <polyline points="9 21 3 21 3 15"></polyline>
                <line x1="21" y1="3" x2="14" y2="10"></line>
                <line x1="3" y1="21" x2="10" y2="14"></line>
            </svg>
            最大化
        `;
        document.body.style.overflow = '';
    }

    // Re-render to adjust positions
    renderNetworkCanvas();
}

function toggleConnectMode() {
    isConnectMode = !isConnectMode;
    connectFromNode = null;
    const btn = document.getElementById('network-connect-mode-btn');
    if (btn) {
        btn.classList.toggle('btn-primary', isConnectMode);
        btn.classList.toggle('btn-secondary', !isConnectMode);
        btn.textContent = isConnectMode ? '🔗 接続モード ON' : '🔗 接続モード';
    }
    document.getElementById('network-instructions').textContent =
        isConnectMode ? '接続モード: ノードをクリックして接続を作成' : 'ドラッグ: ノード移動 | 接続モード: クリックで線を引く';
}

function renderNetworkPalette() {
    const palette = document.getElementById('network-server-palette');
    if (!palette) return;

    palette.innerHTML = servers.map(s => {
        const isProxy = ['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type);
        const bgColor = isProxy ? 'rgba(99, 102, 241, 0.2)' : 'rgba(16, 185, 129, 0.2)';
        const statusColor = s.status === 'Running' ? '#10b981' : '#ef4444';

        return `
            <div style="background: ${bgColor}; padding: 8px 10px; border-radius: 6px; font-size: 0.85em; display: flex; align-items: center; gap: 8px;">
                <div style="width: 8px; height: 8px; border-radius: 50%; background: ${statusColor}; flex-shrink: 0;"></div>
                <div style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                    <div style="font-weight: bold;">${s.name}</div>
                    <div style="font-size: 0.8em; color: #888;">:${s.port} (${s.server_type})</div>
                </div>
            </div>
        `;
    }).join('');
}

function renderNetworkCanvas() {
    const nodesGroup = document.getElementById('network-nodes');
    const connectionsGroup = document.getElementById('network-connections');
    if (!nodesGroup || !connectionsGroup) return;

    // Apply canvas offset for panning
    nodesGroup.setAttribute('transform', `translate(${canvasOffset.x}, ${canvasOffset.y})`);
    connectionsGroup.setAttribute('transform', `translate(${canvasOffset.x}, ${canvasOffset.y})`);

    // Render connections
    connectionsGroup.innerHTML = networkConnections.map((conn, idx) => {
        const fromPos = networkNodes[conn.from];
        const toPos = networkNodes[conn.to];
        if (!fromPos || !toPos) return '';

        return `
            <g class="connection" data-index="${idx}">
                <line x1="${fromPos.x}" y1="${fromPos.y}" x2="${toPos.x}" y2="${toPos.y}" 
                      stroke="#6366f1" stroke-width="2" marker-end="url(#arrowhead)" />
                <circle cx="${(fromPos.x + toPos.x) / 2}" cy="${(fromPos.y + toPos.y) / 2}" r="8" 
                        fill="rgba(239, 68, 68, 0.8)" style="cursor: pointer;" 
                        onclick="removeConnection(${idx})" />
                <text x="${(fromPos.x + toPos.x) / 2}" y="${(fromPos.y + toPos.y) / 2 + 4}" 
                      text-anchor="middle" fill="white" font-size="10" style="pointer-events: none;">×</text>
            </g>
        `;
    }).join('');

    // Render nodes
    nodesGroup.innerHTML = servers.map(s => {
        const pos = networkNodes[s.id] || { x: 100, y: 100 };
        const isProxy = ['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type);
        const fillColor = isProxy ? '#6366f1' : '#10b981';
        const statusColor = s.status === 'Running' ? '#10b981' : '#ef4444';

        return `
            <g class="network-node" data-id="${s.id}" transform="translate(${pos.x}, ${pos.y})" 
               style="cursor: ${isConnectMode ? 'crosshair' : 'grab'};"
               onmousedown="onNodeMouseDown(event, '${s.id}')"
               onclick="onNodeClick(event, '${s.id}')">
                <rect x="-60" y="-25" width="120" height="50" rx="8" 
                      fill="${fillColor}" stroke="${statusColor}" stroke-width="2" />
                <text x="0" y="-3" text-anchor="middle" fill="white" font-size="12" font-weight="bold">${s.name}</text>
                <text x="0" y="12" text-anchor="middle" fill="rgba(255,255,255,0.7)" font-size="10">${s.server_type} :${s.port}</text>
            </g>
        `;
    }).join('');
}

function renderConnectionList() {
    const list = document.getElementById('network-connection-list');
    if (!list) return;

    if (networkConnections.length === 0) {
        list.innerHTML = '<div style="color: #666; text-align: center;">接続がありません</div>';
        return;
    }

    list.innerHTML = networkConnections.map((conn, idx) => {
        const fromServer = servers.find(s => s.id === conn.from);
        const toServer = servers.find(s => s.id === conn.to);
        if (!fromServer || !toServer) return '';

        return `
            <div style="background: rgba(255,255,255,0.05); padding: 8px; border-radius: 4px; margin-bottom: 6px; display: flex; justify-content: space-between; align-items: center;">
                <div>
                    <span style="color: #6366f1;">${fromServer.name}</span>
                    <span style="color: #666;"> → </span>
                    <span style="color: #10b981;">${toServer.name}</span>
                </div>
                <button onclick="removeConnection(${idx})" style="background: rgba(239,68,68,0.2); color: #fca5a5; border: none; padding: 2px 6px; border-radius: 3px; cursor: pointer; font-size: 0.8em;">×</button>
            </div>
        `;
    }).join('');
}

function onNodeMouseDown(event, nodeId) {
    if (isConnectMode) return;
    event.preventDefault();
    event.stopPropagation();

    draggedNode = nodeId;
    const pos = networkNodes[nodeId];
    const canvas = document.getElementById('network-canvas');
    const rect = canvas.getBoundingClientRect();

    dragOffset = {
        x: event.clientX - rect.left - pos.x - canvasOffset.x,
        y: event.clientY - rect.top - pos.y - canvasOffset.y
    };
}

// Start panning when clicking on empty space
function onCanvasMouseDown(event) {
    // If clicking on a node, don't pan
    if (event.target.closest('.network-node')) return;
    if (isConnectMode) return;

    isPanning = true;
    panStart = {
        x: event.clientX - canvasOffset.x,
        y: event.clientY - canvasOffset.y
    };

    const canvas = document.getElementById('network-canvas');
    if (canvas) canvas.style.cursor = 'grabbing';
}

function onCanvasMouseMove(event) {
    // Handle node dragging
    if (draggedNode) {
        const canvas = document.getElementById('network-canvas');
        const rect = canvas.getBoundingClientRect();

        networkNodes[draggedNode] = {
            x: event.clientX - rect.left - dragOffset.x - canvasOffset.x,
            y: event.clientY - rect.top - dragOffset.y - canvasOffset.y
        };

        // Use correct render function based on context
        if (currentProxyNodeId) {
            const pn = proxyNodes.find(p => p.id === currentProxyNodeId);
            if (pn) renderProxyNodeCanvas(pn);
        } else {
            renderNetworkCanvas();
        }
        return;
    }

    // Handle canvas panning
    if (isPanning) {
        canvasOffset = {
            x: event.clientX - panStart.x,
            y: event.clientY - panStart.y
        };

        // Use correct render function based on context
        if (currentProxyNodeId) {
            const pn = proxyNodes.find(p => p.id === currentProxyNodeId);
            if (pn) renderProxyNodeCanvas(pn);
        } else {
            renderNetworkCanvas();
        }
    }
}

function onCanvasMouseUp() {
    draggedNode = null;
    isPanning = false;

    const canvas = document.getElementById('network-canvas');
    if (canvas) canvas.style.cursor = isConnectMode ? 'crosshair' : 'grab';
}

function onNodeClick(event, nodeId) {
    if (!isConnectMode) return;
    event.stopPropagation();

    if (!connectFromNode) {
        connectFromNode = nodeId;
        showNotification(`${servers.find(s => s.id === nodeId)?.name} を選択。次に接続先をクリック`, 'info');
    } else {
        if (connectFromNode !== nodeId) {
            // Check if connection already exists
            const exists = networkConnections.some(c =>
                (c.from === connectFromNode && c.to === nodeId) ||
                (c.from === nodeId && c.to === connectFromNode)
            );

            if (!exists) {
                networkConnections.push({ from: connectFromNode, to: nodeId });
                applyConnectionToProxy(connectFromNode, nodeId);

                // Use correct render function based on context
                if (currentProxyNodeId) {
                    const pn = proxyNodes.find(p => p.id === currentProxyNodeId);
                    if (pn) {
                        renderProxyNodeCanvas(pn);
                        renderProxyNodeConnectionList(pn);
                    }
                } else {
                    renderNetworkCanvas();
                    renderConnectionList();
                }
                showNotification('接続を追加しました', 'success');
            } else {
                showNotification('既に接続されています', 'warning');
            }
        }
        connectFromNode = null;
    }
}

async function applyConnectionToProxy(fromId, toId) {
    const fromServer = servers.find(s => s.id === fromId);
    const toServer = servers.find(s => s.id === toId);

    if (!fromServer || !toServer) return;

    const isProxy = (s) => ['Velocity', 'Waterfall', 'BungeeCord'].includes(s?.server_type);
    const isFromProxy = isProxy(fromServer);
    const isToProxy = isProxy(toServer);

    // Helper: find all proxies connected to this node (traversing through the graph)
    const findConnectedProxies = (startId, visited = new Set()) => {
        if (visited.has(startId)) return [];
        visited.add(startId);

        const proxies = [];
        const server = servers.find(s => s.id === startId);
        if (isProxy(server)) {
            proxies.push(startId);
        }

        // Traverse connections
        networkConnections.forEach(conn => {
            if (conn.from === startId && !visited.has(conn.to)) {
                proxies.push(...findConnectedProxies(conn.to, visited));
            }
            if (conn.to === startId && !visited.has(conn.from)) {
                proxies.push(...findConnectedProxies(conn.from, visited));
            }
        });

        return proxies;
    };

    // If 'from' is a proxy, add 'to' as a backend server (direct connection)
    if (isFromProxy && !isToProxy) {
        try {
            await invoke('add_proxy_server', {
                proxyId: fromId,
                name: toServer.name,
                address: '127.0.0.1:' + toServer.port,
                addToTry: true  // Direct connection to proxy
            });
            await invoke('configure_backend_for_proxy', {
                backendId: toId,
                proxyId: fromId
            });
            console.log(`Added and configured ${toServer.name} for proxy ${fromServer.name}`);
            showNotification(`${toServer.name} をプロキシ用に自動設定しました`, 'success');
        } catch (e) {
            console.error('Failed to configure for proxy:', e);
        }
    } else if (isToProxy && !isFromProxy) {
        try {
            await invoke('add_proxy_server', {
                proxyId: toId,
                name: fromServer.name,
                address: '127.0.0.1:' + fromServer.port,
                addToTry: true  // Direct connection to proxy
            });
            await invoke('configure_backend_for_proxy', {
                backendId: fromId,
                proxyId: toId
            });
            console.log(`Added and configured ${fromServer.name} for proxy ${toServer.name}`);
            showNotification(`${fromServer.name} をプロキシ用に自動設定しました`, 'success');
        } catch (e) {
            console.error('Failed to configure for proxy:', e);
        }
    } else if (!isFromProxy && !isToProxy) {
        // Both are regular servers - find connected proxies and add both to them
        const proxiesFromFrom = findConnectedProxies(fromId);
        const proxiesFromTo = findConnectedProxies(toId);
        const allProxies = [...new Set([...proxiesFromFrom, ...proxiesFromTo])];

        for (const proxyId of allProxies) {
            // Add the server that's NOT already directly connected to the proxy
            const serverToAdd = proxiesFromFrom.includes(proxyId) ? toServer : fromServer;
            const serverId = serverToAdd.id;

            try {
                await invoke('add_proxy_server', {
                    proxyId,
                    name: serverToAdd.name,
                    address: '127.0.0.1:' + serverToAdd.port,
                    addToTry: false  // Indirect connection, don't add to try array
                });
                await invoke('configure_backend_for_proxy', {
                    backendId: serverId,
                    proxyId
                });
                console.log(`Added ${serverToAdd.name} to proxy via tree connection`);
                showNotification(`${serverToAdd.name} をプロキシに追加しました`, 'success');
            } catch (e) {
                console.error('Failed to add to proxy via tree:', e);
            }
        }
    }
}

async function removeConnection(index) {
    const conn = networkConnections[index];
    if (!conn) return;

    const fromServer = servers.find(s => s.id === conn.from);
    const toServer = servers.find(s => s.id === conn.to);

    // Remove from proxy config if applicable
    const isFromProxy = fromServer && ['Velocity', 'Waterfall', 'BungeeCord'].includes(fromServer.server_type);
    const isToProxy = toServer && ['Velocity', 'Waterfall', 'BungeeCord'].includes(toServer.server_type);

    if (isFromProxy && toServer) {
        try {
            await invoke('remove_proxy_server', { proxyId: conn.from, name: toServer.name });
        } catch (e) {
            console.warn('Failed to remove from proxy config:', e);
        }
    } else if (isToProxy && fromServer) {
        try {
            await invoke('remove_proxy_server', { proxyId: conn.to, name: fromServer.name });
        } catch (e) {
            console.warn('Failed to remove from proxy config:', e);
        }
    }

    networkConnections.splice(index, 1);
    renderNetworkCanvas();
    renderConnectionList();
    showNotification('接続を削除しました', 'success');
}

// Keep old functions for backwards compatibility (will be removed later)
let selectedProxyId = null;
async function selectProxy() { }
async function updateNetworkView() { initNetworkEditor(); }
function renderNetworkGraph() { }
function updateAvailableServers() { }
async function linkServer() { }
async function unlinkServer() { }

// Start/Stop All Servers
async function startAllServers() {
    const nonProxies = servers.filter(s => !['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type));
    const proxies = servers.filter(s => ['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type));

    showNotification('全サーバーを起動します...', 'info');

    // Start backend servers first
    for (const s of nonProxies) {
        if (s.status !== 'Running') {
            try {
                await startServer(s.id);
                await new Promise(r => setTimeout(r, 1000)); // Wait 1s between starts
            } catch (e) {
                console.error(`Failed to start ${s.name}:`, e);
            }
        }
    }

    // Then start proxies
    for (const s of proxies) {
        if (s.status !== 'Running') {
            try {
                await startServer(s.id);
                await new Promise(r => setTimeout(r, 1000));
            } catch (e) {
                console.error(`Failed to start ${s.name}:`, e);
            }
        }
    }

    showNotification('全サーバーを起動しました', 'success');
    initNetworkEditor();
}

async function stopAllServers() {
    showNotification('全サーバーを停止します...', 'info');

    // Stop proxies first
    const proxies = servers.filter(s => ['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type));
    for (const s of proxies) {
        if (s.status === 'Running') {
            try {
                await stopServer(s.id);
            } catch (e) {
                console.error(`Failed to stop ${s.name}:`, e);
            }
        }
    }

    // Then stop backend servers
    const nonProxies = servers.filter(s => !['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type));
    for (const s of nonProxies) {
        if (s.status === 'Running') {
            try {
                await stopServer(s.id);
            } catch (e) {
                console.error(`Failed to stop ${s.name}:`, e);
            }
        }
    }

    showNotification('全サーバーを停止しました', 'success');
    initNetworkEditor();
}

// Auto-layout network in a tree structure
function autoLayoutNetwork() {
    const canvas = document.getElementById('network-canvas');
    const rect = canvas?.getBoundingClientRect();
    const width = rect?.width || 800;
    const height = rect?.height || 600;

    // Find proxies (root nodes)
    const proxies = servers.filter(s => ['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type));
    const nonProxies = servers.filter(s => !['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type));

    // Build connection graph
    const getConnectedNodes = (nodeId) => {
        return networkConnections
            .filter(c => c.from === nodeId || c.to === nodeId)
            .map(c => c.from === nodeId ? c.to : c.from);
    };

    // Layout proxies at top
    proxies.forEach((proxy, i) => {
        const xOffset = width / (proxies.length + 1);
        networkNodes[proxy.id] = {
            x: xOffset * (i + 1),
            y: 60
        };
    });

    // Find servers directly connected to proxies (level 1)
    const level1 = new Set();
    const level2 = new Set();

    proxies.forEach(proxy => {
        const connected = getConnectedNodes(proxy.id);
        connected.forEach(id => {
            if (!proxies.find(p => p.id === id)) {
                level1.add(id);
            }
        });
    });

    // Find servers connected to level1 servers (level 2)
    level1.forEach(id => {
        const connected = getConnectedNodes(id);
        connected.forEach(cid => {
            if (!proxies.find(p => p.id === cid) && !level1.has(cid)) {
                level2.add(cid);
            }
        });
    });

    // Position level 1 servers
    const level1Array = Array.from(level1);
    level1Array.forEach((id, i) => {
        const xOffset = width / (level1Array.length + 1);
        networkNodes[id] = {
            x: xOffset * (i + 1),
            y: 200
        };
    });

    // Position level 2 servers
    const level2Array = Array.from(level2);
    level2Array.forEach((id, i) => {
        const xOffset = width / (level2Array.length + 1);
        networkNodes[id] = {
            x: xOffset * (i + 1),
            y: 350
        };
    });

    // Position remaining unconnected servers
    const positioned = new Set([...proxies.map(p => p.id), ...level1, ...level2]);
    const unpositioned = nonProxies.filter(s => !positioned.has(s.id));
    unpositioned.forEach((s, i) => {
        const xOffset = width / (unpositioned.length + 1);
        networkNodes[s.id] = {
            x: xOffset * (i + 1),
            y: 500
        };
    });

    renderNetworkCanvas();
    showNotification('自動配置しました', 'success');
}

// =============================================
// ProxyNode Creation Modal
// =============================================

function openProxyNodeModal() {
    const modal = document.getElementById('proxynode-modal');
    if (!modal) return;

    // Populate proxy dropdown with existing proxy servers
    const proxySelect = document.getElementById('proxynode-proxy-select');
    const proxies = servers.filter(s => ['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type));

    proxySelect.innerHTML = `<option value="">-- 選択してください --</option>` +
        proxies.map(p => `<option value="${p.id}">${p.name} (${p.server_type} :${p.port})</option>`).join('');

    if (proxies.length === 0) {
        proxySelect.innerHTML = `<option value="">プロキシサーバーがありません</option>`;
    }

    // Populate backend server checklist (non-proxy servers)
    const backendList = document.getElementById('proxynode-backend-list');
    const backends = servers.filter(s => !['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type));

    if (backends.length === 0) {
        backendList.innerHTML = '<div style="color: #888; text-align: center;">バックエンドサーバーがありません</div>';
    } else {
        backendList.innerHTML = backends.map(s => `
            <label class="checkbox-item" style="display: flex; align-items: center; gap: 10px; padding: 8px; background: rgba(255,255,255,0.05); border-radius: 4px; margin-bottom: 6px; cursor: pointer;">
                <input type="checkbox" class="proxynode-backend-checkbox" value="${s.id}" data-name="${s.name}" data-port="${s.port}">
                <span style="flex: 1;">
                    <strong>${s.name}</strong>
                    <span style="color: #888; font-size: 0.85em; margin-left: 8px;">${s.server_type} :${s.port}</span>
                </span>
                <span class="server-status ${s.status.toLowerCase()}" style="font-size: 0.8em;">${s.status}</span>
            </label>
        `).join('');
    }

    modal.classList.remove('hidden');
    requestAnimationFrame(() => modal.classList.add('active'));
}

function closeProxyNodeModal() {
    const modal = document.getElementById('proxynode-modal');
    if (modal) {
        modal.classList.remove('active');
        setTimeout(() => modal.classList.add('hidden'), 300);
    }

    // Reset modal state
    isAddingBackend = false;

    // Restore proxy selector visibility
    const proxyGroup = document.getElementById('proxynode-proxy-select')?.parentElement;
    if (proxyGroup) proxyGroup.style.display = '';

    // Restore button text
    const confirmBtn = document.getElementById('confirm-proxynode-btn');
    if (confirmBtn) confirmBtn.textContent = '作成';
}

async function createProxyNode() {
    const proxyId = document.getElementById('proxynode-proxy-select').value;
    const checkboxes = document.querySelectorAll('.proxynode-backend-checkbox:checked');

    if (!proxyId) {
        showNotification('プロキシサーバーを選択してください', 'error');
        return;
    }

    // Only require backends when adding to existing ProxyNode
    if (isAddingBackend && checkboxes.length === 0) {
        showNotification('バックエンドサーバーを1つ以上選択してください', 'error');
        return;
    }

    const proxy = servers.find(s => s.id === proxyId);
    const backendIds = Array.from(checkboxes).map(cb => ({
        id: cb.value,
        name: cb.dataset.name,
        port: cb.dataset.port
    }));

    showNotification(isAddingBackend ? 'バックエンドを追加中...' : 'ProxyNodeを作成中...', 'info');

    try {
        // Add each backend to the proxy
        for (const backend of backendIds) {
            await invoke('add_proxy_server', {
                proxyId,
                name: backend.name,
                address: '127.0.0.1:' + backend.port,
                addToTry: true
            });

            await invoke('configure_backend_for_proxy', {
                backendId: backend.id,
                proxyId
            });
        }

        if (isAddingBackend) {
            // Adding to existing ProxyNode
            const pn = proxyNodes.find(p => p.id === currentProxyNodeId);
            if (pn) {
                backendIds.forEach(b => {
                    if (!pn.backends.includes(b.id)) {
                        pn.backends.push(b.id);
                    }
                });
                saveProxyNodes();
            }
            closeProxyNodeModal();
            showNotification(`${backendIds.length}個のサーバーを追加しました`, 'success');
            showProxyNodeDetail(currentProxyNodeId);
        } else {
            // Creating new ProxyNode
            const newProxyNode = {
                id: crypto.randomUUID(),
                name: proxy.name,
                proxyId: proxyId,
                backends: backendIds.map(b => b.id)
            };
            proxyNodes.push(newProxyNode);
            saveProxyNodes();
            closeProxyNodeModal();
            showNotification(`ProxyNode作成完了: ${proxy.name} → ${backendIds.length}サーバー`, 'success');
            renderProxyNodeCards();
        }

    } catch (e) {
        console.error('Failed to create/update ProxyNode:', e);
        showNotification(`失敗: ${e}`, 'error');
    }
}

// =============================================
// ProxyNode Data Management
// =============================================

let proxyNodes = []; // Array of ProxyNode objects
let currentProxyNodeId = null;
let isAddingBackend = false; // Flag to track if adding to existing ProxyNode

function loadProxyNodes() {
    try {
        const saved = localStorage.getItem('proxyNodes');
        if (saved) {
            proxyNodes = JSON.parse(saved);
        }
    } catch (e) {
        console.warn('Failed to load proxyNodes:', e);
        proxyNodes = [];
    }
}

function saveProxyNodes() {
    try {
        localStorage.setItem('proxyNodes', JSON.stringify(proxyNodes));
    } catch (e) {
        console.error('Failed to save proxyNodes:', e);
    }
}

function renderProxyNodeCards() {
    const container = document.getElementById('proxynode-list');
    if (!container) return;

    if (proxyNodes.length === 0) {
        container.innerHTML = `
            <div class="empty-state">
                <h3>ProxyNodeがありません</h3>
                <p>「+ ProxyNode作成」から追加してください。</p>
            </div>
        `;
        return;
    }

    container.innerHTML = proxyNodes.map(pn => {
        const proxy = servers.find(s => s.id === pn.proxyId);
        const backendCount = pn.backends.length;
        const proxyStatus = proxy?.status || 'Unknown';
        const statusClass = proxyStatus.toLowerCase();

        return `
            <div class="server-card">
                <div class="server-card-header">
                    <h3 class="server-name">${escapeHtml(pn.name)}</h3>
                    <span class="server-status ${statusClass}">${proxyStatus}</span>
                </div>
                <div class="server-info">
                    <div class="server-info-item"><span>タイプ:</span><span>${proxy?.server_type || '-'}</span></div>
                    <div class="server-info-item"><span>ポート:</span><span>${proxy?.port || '-'}</span></div>
                    <div class="server-info-item"><span>サーバー数:</span><span>${backendCount}</span></div>
                </div>
                <div class="server-actions">
                    <button class="btn btn-primary btn-sm" onclick="showProxyNodeDetail('${pn.id}')">詳細</button>
                    <button class="btn btn-secondary btn-sm" onclick="deleteProxyNodeById('${pn.id}')">削除</button>
                </div>
            </div>
        `;
    }).join('');
}

function showProxyNodeDetail(id) {
    const pn = proxyNodes.find(p => p.id === id);
    if (!pn) return;

    currentProxyNodeId = id;
    switchView('proxynode-detail');

    const proxy = servers.find(s => s.id === pn.proxyId);

    // Update header
    document.getElementById('proxynode-detail-name').textContent = pn.name;
    const statusEl = document.getElementById('proxynode-detail-status');
    statusEl.textContent = proxy?.status || 'Unknown';
    statusEl.className = `server-status ${(proxy?.status || 'stopped').toLowerCase()}`;

    // Initialize NodeUI with this ProxyNode's servers
    initProxyNodeEditor(pn);
}

// Initialize NodeUI canvas for a specific ProxyNode
function initProxyNodeEditor(pn) {
    const proxy = servers.find(s => s.id === pn.proxyId);
    if (!proxy) return;

    // Get all servers for this ProxyNode (proxy + backends)
    const proxyNodeServers = [proxy, ...pn.backends.map(id => servers.find(s => s.id === id)).filter(Boolean)];

    // Load saved positions for this ProxyNode
    const savedLayout = localStorage.getItem(`proxyNodeLayout_${pn.id}`);
    if (savedLayout) {
        try {
            const data = JSON.parse(savedLayout);
            networkNodes = data.nodes || {};
            networkConnections = data.connections || [];
        } catch (e) {
            networkNodes = {};
            networkConnections = [];
        }
    } else {
        networkNodes = {};
        networkConnections = [];
    }

    // Reset pan offset
    canvasOffset = { x: 0, y: 0 };

    // Initialize positions for servers not yet placed
    const canvasRect = document.getElementById('network-canvas')?.getBoundingClientRect();
    const centerX = (canvasRect?.width || 800) / 2;
    const centerY = (canvasRect?.height || 500) / 2;

    // Place proxy at center top
    if (!networkNodes[proxy.id]) {
        networkNodes[proxy.id] = { x: centerX, y: 100 };
    }

    // Place backends in a row below (only those explicitly in pn.backends)
    const backends = pn.backends.map(id => servers.find(s => s.id === id)).filter(Boolean);
    backends.forEach((s, i) => {
        if (!networkNodes[s.id]) {
            const xOffset = (canvasRect?.width || 800) / (backends.length + 1);
            networkNodes[s.id] = {
                x: xOffset * (i + 1),
                y: 300
            };
        }
    });

    // Don't auto-create connections - let user create them via connect mode

    // Render palette with available servers to add
    renderProxyNodePalette(pn);

    // Render canvas with only this ProxyNode's servers
    renderProxyNodeCanvas(pn);

    // Render connection list
    renderProxyNodeConnectionList(pn);
}

// Render server palette for adding to ProxyNode
function renderProxyNodePalette(pn) {
    const palette = document.getElementById('network-server-palette');
    if (!palette) return;

    // Show servers not yet in this ProxyNode
    const availableServers = servers.filter(s =>
        !['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type) &&
        !pn.backends.includes(s.id)
    );

    if (availableServers.length === 0) {
        palette.innerHTML = '<div style="color: #666; text-align: center; font-size: 0.85em;">追加可能なサーバーなし</div>';
        return;
    }

    palette.innerHTML = availableServers.map(s => `
        <div onclick="addServerToProxyNode('${s.id}')" style="background: rgba(16, 185, 129, 0.2); padding: 8px 10px; border-radius: 6px; font-size: 0.85em; cursor: pointer; display: flex; align-items: center; gap: 8px;">
            <div style="width: 8px; height: 8px; border-radius: 50%; background: ${s.status === 'Running' ? '#10b981' : '#ef4444'}; flex-shrink: 0;"></div>
            <div style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                <div style="font-weight: bold;">${escapeHtml(s.name)}</div>
                <div style="font-size: 0.8em; color: #888;">:${s.port}</div>
            </div>
        </div>
    `).join('');
}

// Render canvas for ProxyNode
function renderProxyNodeCanvas(pn) {
    const nodesGroup = document.getElementById('network-nodes');
    const connectionsGroup = document.getElementById('network-connections');
    if (!nodesGroup || !connectionsGroup) return;

    const proxy = servers.find(s => s.id === pn.proxyId);
    const proxyNodeServers = [proxy, ...pn.backends.map(id => servers.find(s => s.id === id)).filter(Boolean)];

    // Apply canvas offset for panning
    nodesGroup.setAttribute('transform', `translate(${canvasOffset.x}, ${canvasOffset.y})`);
    connectionsGroup.setAttribute('transform', `translate(${canvasOffset.x}, ${canvasOffset.y})`);

    // Render connections
    connectionsGroup.innerHTML = networkConnections.map((conn, idx) => {
        const fromPos = networkNodes[conn.from];
        const toPos = networkNodes[conn.to];
        if (!fromPos || !toPos) return '';

        return `
            <g class="connection" data-index="${idx}">
                <line x1="${fromPos.x}" y1="${fromPos.y}" x2="${toPos.x}" y2="${toPos.y}" 
                      stroke="#6366f1" stroke-width="2" marker-end="url(#arrowhead)" />
            </g>
        `;
    }).join('');

    // Render nodes
    nodesGroup.innerHTML = proxyNodeServers.map(s => {
        if (!s) return '';
        const pos = networkNodes[s.id] || { x: 100, y: 100 };
        const isProxy = ['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type);
        const fillColor = isProxy ? '#6366f1' : '#10b981';
        const statusColor = s.status === 'Running' ? '#10b981' : '#ef4444';

        return `
            <g class="network-node" data-id="${s.id}" transform="translate(${pos.x}, ${pos.y})" 
               style="cursor: ${isConnectMode ? 'crosshair' : 'grab'};"
               onmousedown="onNodeMouseDown(event, '${s.id}')"
               onclick="onNodeClick(event, '${s.id}')">
                <rect x="-60" y="-25" width="120" height="50" rx="8" 
                      fill="${fillColor}" stroke="${statusColor}" stroke-width="2" />
                <text x="0" y="-3" text-anchor="middle" fill="white" font-size="12" font-weight="bold">${escapeHtml(s.name)}</text>
                <text x="0" y="12" text-anchor="middle" fill="rgba(255,255,255,0.7)" font-size="10">${s.server_type} :${s.port}</text>
            </g>
        `;
    }).join('');
}

// Render connection list for ProxyNode
function renderProxyNodeConnectionList(pn) {
    const list = document.getElementById('network-connection-list');
    if (!list) return;

    const proxy = servers.find(s => s.id === pn.proxyId);
    const backends = pn.backends.map(id => servers.find(s => s.id === id)).filter(Boolean);

    if (backends.length === 0) {
        list.innerHTML = '<div style="color: #666; text-align: center;">バックエンドなし</div>';
        return;
    }

    list.innerHTML = backends.map(backend => `
        <div style="background: rgba(255,255,255,0.05); padding: 8px; border-radius: 4px; margin-bottom: 6px; display: flex; justify-content: space-between; align-items: center;">
            <div>
                <span style="color: #6366f1;">${escapeHtml(proxy?.name || '')}</span>
                <span style="color: #666;"> → </span>
                <span style="color: #10b981;">${escapeHtml(backend.name)}</span>
            </div>
            <button onclick="removeServerFromProxyNode('${backend.id}')" style="background: rgba(239,68,68,0.2); color: #fca5a5; border: none; padding: 2px 6px; border-radius: 3px; cursor: pointer; font-size: 0.8em;">×</button>
        </div>
    `).join('');
}

// Add server to current ProxyNode
async function addServerToProxyNode(serverId) {
    const pn = proxyNodes.find(p => p.id === currentProxyNodeId);
    if (!pn || pn.backends.includes(serverId)) return;

    const server = servers.find(s => s.id === serverId);
    if (!server) return;

    try {
        await invoke('add_proxy_server', {
            proxyId: pn.proxyId,
            name: server.name,
            address: '127.0.0.1:' + server.port,
            addToTry: true
        });

        await invoke('configure_backend_for_proxy', {
            backendId: serverId,
            proxyId: pn.proxyId
        });

        pn.backends.push(serverId);
        saveProxyNodes();

        // Add to canvas
        const canvasRect = document.getElementById('network-canvas')?.getBoundingClientRect();
        networkNodes[serverId] = {
            x: (canvasRect?.width || 800) / 2 + Math.random() * 100 - 50,
            y: 300 + Math.random() * 100
        };
        networkConnections.push({ from: pn.proxyId, to: serverId });

        initProxyNodeEditor(pn);
        showNotification(`${server.name} を追加しました`, 'success');
    } catch (e) {
        console.error('Failed to add server:', e);
        showNotification('追加に失敗しました', 'error');
    }
}

// Remove server from current ProxyNode
async function removeServerFromProxyNode(serverId) {
    const pn = proxyNodes.find(p => p.id === currentProxyNodeId);
    if (!pn) return;

    const server = servers.find(s => s.id === serverId);
    if (!server) return;

    try {
        await invoke('remove_proxy_server', {
            proxyId: pn.proxyId,
            name: server.name
        });

        pn.backends = pn.backends.filter(id => id !== serverId);
        saveProxyNodes();

        // Remove from canvas
        delete networkNodes[serverId];
        networkConnections = networkConnections.filter(c => c.from !== serverId && c.to !== serverId);

        initProxyNodeEditor(pn);
        showNotification(`${server.name} を削除しました`, 'success');
    } catch (e) {
        console.error('Failed to remove server:', e);
        showNotification('削除に失敗しました', 'error');
    }
}

// Save layout for current ProxyNode
function saveProxyNodeLayout() {
    const pn = proxyNodes.find(p => p.id === currentProxyNodeId);
    if (!pn) return;

    try {
        localStorage.setItem(`proxyNodeLayout_${pn.id}`, JSON.stringify({
            nodes: networkNodes,
            connections: networkConnections
        }));
        showNotification('レイアウトを保存しました', 'success');
    } catch (e) {
        console.error('Failed to save layout:', e);
    }
}

function openAddBackendModal() {
    const pn = proxyNodes.find(p => p.id === currentProxyNodeId);
    if (!pn) return;

    // Set mode to "adding to existing"
    isAddingBackend = true;

    // Reuse existing modal but filter out already added backends
    const modal = document.getElementById('proxynode-modal');
    if (!modal) return;

    // Hide proxy selector (we already know which proxy)
    const proxyGroup = document.getElementById('proxynode-proxy-select').parentElement;
    proxyGroup.style.display = 'none';
    document.getElementById('proxynode-proxy-select').value = pn.proxyId;

    // Change button text
    const confirmBtn = document.getElementById('confirm-proxynode-btn');
    if (confirmBtn) confirmBtn.textContent = '追加';

    // Populate backend list, excluding already added
    const backendListContainer = document.getElementById('proxynode-backend-list');
    const backends = servers.filter(s =>
        !['Velocity', 'Waterfall', 'BungeeCord'].includes(s.server_type) &&
        !pn.backends.includes(s.id)
    );

    if (backends.length === 0) {
        backendListContainer.innerHTML = '<div style="color: #888; text-align: center;">追加可能なサーバーがありません</div>';
    } else {
        backendListContainer.innerHTML = backends.map(s => `
            <label class="checkbox-item" style="display: flex; align-items: center; gap: 10px; padding: 8px; background: rgba(255,255,255,0.05); border-radius: 4px; margin-bottom: 6px; cursor: pointer;">
                <input type="checkbox" class="proxynode-backend-checkbox" value="${s.id}" data-name="${s.name}" data-port="${s.port}">
                <span style="flex: 1;">
                    <strong>${s.name}</strong>
                    <span style="color: #888; font-size: 0.85em; margin-left: 8px;">${s.server_type} :${s.port}</span>
                </span>
                <span class="server-status ${s.status.toLowerCase()}" style="font-size: 0.8em;">${s.status}</span>
            </label>
        `).join('');
    }

    modal.classList.remove('hidden');
    requestAnimationFrame(() => modal.classList.add('active'));
}

async function deleteProxyNodeById(id) {
    const pn = proxyNodes.find(p => p.id === id);
    if (!pn) return;

    const confirmed = await showConfirmModal(`ProxyNode「${pn.name}」を削除しますか？`);
    if (!confirmed) return;

    proxyNodes = proxyNodes.filter(p => p.id !== id);
    saveProxyNodes();
    renderProxyNodeCards();
    showNotification('ProxyNodeを削除しました', 'success');
}

async function deleteProxyNode() {
    if (currentProxyNodeId) {
        await deleteProxyNodeById(currentProxyNodeId);
        switchView('network');
    }
}

async function startProxyNode() {
    const pn = proxyNodes.find(p => p.id === currentProxyNodeId);
    if (!pn) return;

    showNotification('ProxyNodeを起動中...', 'info');

    // Start backends first
    for (const backendId of pn.backends) {
        const server = servers.find(s => s.id === backendId);
        if (server && server.status !== 'Running') {
            try {
                await startServer(backendId);
                await new Promise(r => setTimeout(r, 500));
            } catch (e) {
                console.error(`Failed to start ${server.name}:`, e);
            }
        }
    }

    // Then start proxy
    const proxy = servers.find(s => s.id === pn.proxyId);
    if (proxy && proxy.status !== 'Running') {
        try {
            await startServer(pn.proxyId);
        } catch (e) {
            console.error(`Failed to start proxy:`, e);
        }
    }

    await loadServers();
    showProxyNodeDetail(currentProxyNodeId);
    showNotification('ProxyNodeを起動しました', 'success');
}

async function stopProxyNode() {
    const pn = proxyNodes.find(p => p.id === currentProxyNodeId);
    if (!pn) return;

    showNotification('ProxyNodeを停止中...', 'info');

    // Stop proxy first
    const proxy = servers.find(s => s.id === pn.proxyId);
    if (proxy && proxy.status === 'Running') {
        try {
            await stopServer(pn.proxyId);
        } catch (e) {
            console.error(`Failed to stop proxy:`, e);
        }
    }

    // Then stop backends
    for (const backendId of pn.backends) {
        const server = servers.find(s => s.id === backendId);
        if (server && server.status === 'Running') {
            try {
                await stopServer(backendId);
            } catch (e) {
                console.error(`Failed to stop ${server.name}:`, e);
            }
        }
    }

    await loadServers();
    showProxyNodeDetail(currentProxyNodeId);
    showNotification('ProxyNodeを停止しました', 'success');
}

// Initialize ProxyNodes on network tab
document.addEventListener('DOMContentLoaded', () => {
    const networkBtn = document.querySelector('.nav-item[data-view="network"]');
    if (networkBtn) {
        networkBtn.addEventListener('click', () => {
            loadProxyNodes();
            renderProxyNodeCards();
        });
    }
});
