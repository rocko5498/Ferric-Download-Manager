import QtQuick
import QtQuick.Controls
import QtQuick.Window
import Qt.labs.platform as Platform
import com.ferric.app 1.0

Window {
    id: root

    onClosing: function(close) {
        // Closing the window (X button) hides to tray instead of quitting \u2014
        // Quit is only reachable via the tray menu, matching "Quit exits
        // cleanly" being a deliberate action, not an accidental window close.
        if (root.quitting) return
        // With no tray icon (or "close to tray" switched off) nothing can bring the window back, so the X
        // must really quit; it used to hide anyway and look like a dead button.
        if (!root.showTrayIcon || root.prefsData["close_to_tray"] === "0") { root.quitApp(); return }
        close.accepted = false
        root.visible = false
    }
    property bool quitting: false
    // Qt.quit() closes every window first; onClosing must let that through or the quit is cancelled.
    function quitApp() { quitting = true; Qt.quit(); }
    width: 1000
    height: 650
    // Started at login with --autostart: begin hidden, then loadPreferences() shows the window unless
    // "Start minimized to system tray" is on (the settings are only known once the daemon answers).
    readonly property bool autostartLaunch: Qt.application.arguments.indexOf("--autostart") >= 0
    property bool autostartResolved: false
    visible: !autostartLaunch
    color: Theme.bg
    title: qsTr("Ferric")

    DaemonSession {
        id: session
        // The daemon died (crash or kill): bring it back the way a cold start does.
        onConnectedChanged: if (connected) daemonRestartTimer.stop(); else daemonRestartTimer.start()
    }

    Timer {
        id: daemonRestartTimer
        interval: 4000
        repeat: true
        onTriggered: session.refresh()
    }

    ProgressHub {
        id: hub
    }

    DownloadModel {
        id: downloadModel
    }

    TorrentModel {
        id: torrentModel
    }

    property int nextBusRequestId: 0
    property var busCallbacks: ({})

    function requestBus(operation, args, callback) {
        var requestId = "qml-" + (++root.nextBusRequestId);
        root.busCallbacks[requestId] = callback;
        bus.request(requestId, operation, JSON.stringify(args || {}));
    }

    // Error text from a bus result: `{"error": ...}` JSON, or a plain non-JSON
    // string (operations that return nothing on success). "" means success.
    function busError(raw) {
        if (!raw) return "";
        try {
            var r = JSON.parse(raw);
            return (r && r.error) ? String(r.error) : "";
        } catch (e) {
            return String(raw);
        }
    }

    function requestBusBatch(operation, argsList, callback) {
        if (argsList.length === 0) {
            callback([]);
            return;
        }
        var results = new Array(argsList.length);
        var remaining = argsList.length;
        for (var i = 0; i < argsList.length; i++) {
            (function(index) {
                requestBus(operation, argsList[index], function(result) {
                    results[index] = result;
                    remaining--;
                    if (remaining === 0)
                        callback(results);
                });
            })(i);
        }
    }

    CommandBus {
        id: bus
        onCommandFinished: function(requestId, result) {
            var callback = root.busCallbacks[requestId];
            if (!callback)
                return;
            delete root.busCallbacks[requestId];
            callback(result);
        }
    }

    Component.onCompleted: {
        Qt.callLater(function() { root.refreshGrabProjects(); });
        startupTimer.start();
    }

    Timer {
        id: startupTimer
        interval: 0
        repeat: false
        onTriggered: {
            session.refresh()
            hub.start()
            downloadModel.refresh()
            torrentModel.refresh()
            root.reportListFetchErrors()
            loadCategories()
            loadQueues()
            loadRssFeeds();
            loadRssRules();
            loadPreferences();
            loadOverview();
            // S7: retry overview stats after daemon settles (startup call may fail)
            overviewRetryTimer.start();
            // FQT-049: check ferric-host registration status
            requestBus("get_ferric_host_status", {}, function(raw) {
                try {
                    root.ferricHostRegistered = JSON.parse(raw).registered === true;
                } catch(e) { root.ferricHostRegistered = false; }
            });
        }
    }

    // The startup loads above race the daemon spawn on a cold start. Load again as
    // soon as the daemon is reachable instead of leaving the UI half empty.
    Connections {
        target: session
        function onConnectedChanged() {
            if (!session.connected) return;
            downloadModel.refresh();
            torrentModel.refresh();
            root.loadPreferences();
            root.loadOverview();
            root.loadCategories();
            root.loadQueues();
            root.loadRssFeeds();
            root.loadRssRules();
        }
    }

    // S7: retry overview stats once after daemon settles — startup call may return error JSON
    Timer {
        id: overviewRetryTimer
        interval: 2000
        repeat: false
        onTriggered: loadOverview()
    }

    // Full beginResetModel on every SSE tick while ListView is scrolling/resizing
    // hard-crashes Qt on Windows. Debounce list reloads; status-bar speeds still
    // update live via hub property bindings.
    Timer {
        id: modelRefreshDebounce
        interval: 1500
        repeat: false
        onTriggered: {
            // ping = health only (no 10s spawn wait). Cold start still uses session.refresh().
            session.ping()
            downloadModel.refresh()
            torrentModel.refresh()
            root.refreshStatusStats()
            root.recomputeTags()
            root.reportListFetchErrors()
        }
    }

    Connections {
        target: hub
        // this app keeps the Rust (snake_case) signal names, so the handler is onCapture_held
        function onCapture_held(id) { root.openInfoDialog(id) }
        function onCapture_updated(id) { var w = root.infoDialogs[id]; if (w) w.refreshProbe(); }
        function onLogin_required(id) { root.loginIsProxy = false; root.loginDownloadId = id; loginUser.text = ""; loginPass.text = ""; loginDialog.visible = true; loginUser.forceActiveFocus() }
        function onProxy_login_required(id) { root.loginIsProxy = true; root.loginDownloadId = id; loginUser.text = ""; loginPass.text = ""; loginDialog.visible = true; loginUser.forceActiveFocus() }
        function onDownload_done(id) { if (root.prefsData["sound_on_each_done"] === "1") bus.play_completion_sound() }
        function onDownload_failed(id) { if (root.prefsData["sound_on_error"] === "1") bus.play_error_sound() }
        function onShow_ui() { root.visible = true; root.raise(); root.requestActivate(); }
        function onLiveChanged() { if (hub.live) root.openPendingCaptures() }
        function onUpdated() {
            var prevActive = root.lastActiveCount;
            var curActive = hub.active_downloads || 0;
            root.lastActiveCount = curActive;

            if (!modelRefreshDebounce.running)
                modelRefreshDebounce.start()

            // FQT-074: collect speed samples for graph
            if (root.showSpeedGraph) {
                var newHistory = root.speedHistory.slice();
                newHistory.push(hub.download_bps || 0);
                if (newHistory.length > 60) newHistory.shift();
                root.speedHistory = newHistory;
            }
            if (prevActive > curActive && root.anyWhenDoneConfigured())
                whenDoneCheck.restart();
            if (root.prefsData["sound_on_complete"] === "1" && prevActive > 0 && curActive === 0)
                bus.play_completion_sound();
            // FQT-087: notification on completion
            if (root.prefsData["notifications_enabled"] !== "0" && prevActive > 0 && curActive === 0) {
                try {
                    systemTray.showMessage("Ferric", "All downloads complete", Platform.SystemTrayIcon.Information);
                } catch (e) { /* tray toast can fail on some shells */ }
            }
        }
    }

    property int lastActiveCount: 0
    property string lastKillSwitchState: "0"
    // Phase 3 privacy UI: live flag for banner + prefs indicator (FQT-087 poll path).
    readonly property bool killSwitchActive: lastKillSwitchState === "1"

    function applyKillSwitchState(ksLive) {
        if (root.torrentsEnabled && ksLive === "1" && root.lastKillSwitchState !== "1") {
            systemTray.showMessage("Ferric", "Torrent kill switch activated \u2014 torrents paused, proxy unreachable.", Platform.SystemTrayIcon.Warning);
        }
        root.lastKillSwitchState = ksLive;
        // Keep prefsData in sync so PrefsDialog indicator is not stuck on cold-start snapshot.
        if ((root.prefsData["torrent_kill_switch_active"] || "0") !== ksLive) {
            var pd = {};
            for (var k in root.prefsData)
                pd[k] = root.prefsData[k];
            pd["torrent_kill_switch_active"] = ksLive;
            root.prefsData = pd;
        }
    }

    Timer {
        interval: 5000
        running: true
        repeat: true
        onTriggered: {
            requestBus("get_settings", {}, function(raw) {
                var ksLive = "0";
                try { ksLive = (JSON.parse(raw)["torrent_kill_switch_active"] || "0"); } catch(e) {}
                root.applyKillSwitchState(ksLive);
            });
        }
    }

    // ── System tray (FQT-015) ───────────────────────────────────────────
    Platform.SystemTrayIcon {
        id: systemTray
        visible: root.showTrayIcon
        tooltip: "Ferric"
        icon.source: "qrc:/icons/ferric-tray.png"

        onActivated: {
            root.visible = !root.visible
            if (root.visible) {
                root.raise()
                root.requestActivate()
            }
        }

        menu: Platform.Menu {
            Platform.MenuItem {
                text: root.visible ? "Hide" : "Show"
                onTriggered: {
                    root.visible = !root.visible
                    if (root.visible) {
                        root.raise()
                        root.requestActivate()
                    }
                }
            }
            Platform.MenuItem {
                text: qsTr("Quit")
                onTriggered: root.showExitDialog()
            }
        }
    }

    property int workspaceIndex: 0
    // Torrent UI is hidden until it ships; the backend is untouched. Flip to true to show it again.
    readonly property bool torrentsEnabled: false
    readonly property var workspaceNames: ["Overview", "Downloads", "Torrents", "Grabber", "Media", "RSS"]

    // ── Download selection + enablement (FQT-020) ───────────────────────
    // Each entry is { id, status }. Selection lives here (not in the model)
    // since QAbstractListModel has no per-row "selected" role wired yet \u2014
    // full reset on every refresh() would drop it anyway.
    property var selectedDownloads: []
    property var selectedTorrents: []

    function isDownloadSelected(rowId) {
        for (var i = 0; i < root.selectedDownloads.length; i++) {
            if (root.selectedDownloads[i].id === rowId) return true;
        }
        return false;
    }

    function toggleDownloadSelection(rowId, rowStatus, rowName, rowSavePath, rowUrlHost, additive) {
        var next = additive ? root.selectedDownloads.slice() : [];
        var idx = -1;
        for (var i = 0; i < next.length; i++) {
            if (next[i].id === rowId) { idx = i; break; }
        }
        if (idx >= 0) {
            next.splice(idx, 1);
        } else {
            next.push({ id: rowId, status: rowStatus, name: rowName, savePath: rowSavePath, urlHost: rowUrlHost });
        }
        root.selectedDownloads = next;
    }

    function isTorrentSelected(rowId) {
        for (var i = 0; i < root.selectedTorrents.length; i++) {
            if (root.selectedTorrents[i].id === rowId) return true;
        }
        return false;
    }

    function toggleTorrentSelection(rowId, rowStatus, additive) {
        var next = additive ? root.selectedTorrents.slice() : [];
        var idx = -1;
        for (var i = 0; i < next.length; i++) {
            if (next[i].id === rowId) { idx = i; break; }
        }
        if (idx >= 0) {
            next.splice(idx, 1);
        } else {
            next.push({ id: rowId, status: rowStatus });
        }
        root.selectedTorrents = next;
    }

    readonly property bool selectionCanPause: root.selectedDownloads.length > 0 &&
        root.selectedDownloads.every(function(s) {
            return s.status === "queued" || s.status === "connecting" || s.status === "downloading";
        })
    readonly property bool selectionCanResume: root.selectedDownloads.length > 0 &&
        root.selectedDownloads.every(function(s) {
            return s.status === "paused" || s.status === "error";
        })
    readonly property bool selectionCanStop: root.selectedDownloads.length > 0 &&
        root.selectedDownloads.every(function(s) {
            return s.status === "queued" || s.status === "connecting" ||
                   s.status === "downloading" || s.status === "paused" || s.status === "error";
        })

    property var pendingDeleteIds: []

    // FQT-044: categories
    property var categories: []
    property string categoryFilter: ""

    function loadCategories() {
        requestBus("get_category_folders", {}, function(raw) {
            try {
                var value = JSON.parse(raw);
                // General (everything uncategorised) is "All downloads"; the rest are listed.
                root.categories = value.error ? [] : value
                    .filter(function(c) { return c.name !== "General"; })
                    .map(function(c) { return { id: c.name, name: c.name, dir: c.dir, pattern: "" }; });
            } catch(e) { root.categories = []; }
        });
    }

    function doAddCategory(name) {
        if (name.trim() === "") { showError("Category name required"); return; }
        requestBus("add_category", { name: name.trim() }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { showError(res.error); return; }
                addCategoryDialog.visible = false;
                loadCategories();
            } catch(e) { showError("Add category failed: " + e); }
        });
    }

    // FQT-045: queues
    property var queues: []
    property bool hideCategories: false
    readonly property bool toolbarSmall: root.prefsData["toolbar_small"] === "1"
    readonly property bool showTrayIcon: root.prefsData["show_tray_icon"] !== "0"
    function setPref(key, value) { root.savePref(key, value) }
    Platform.FontDialog {
        id: uiFontDialog
        onAccepted: root.setPref("ui_font", font.family)
    }
    // View > Font: one family for the whole interface (sizes stay as designed).
    readonly property string uiFontFamily: root.prefsData["ui_font"] || ""
    onUiFontFamilyChanged: { var f = Qt.application.font; if (uiFontFamily !== "") f.family = uiFontFamily; Qt.application.font = f }
    property int downloadSortKey: 0
    function setDownloadSort(k) { root.downloadSortKey = k; downloadModel.set_sort(k) }
    property string queueFilter: ""

    // FQT-069: tags (live-computed from torrent model)
    property var tags: []
    property string tagFilter: ""
    property string torrentStatusFilter: ""

    function recomputeTags() {
        root.tags = torrentModel.distinct_tags();
    }

    // FQT-074: speed graph + log
    property var speedHistory: []
    property var logLines: []

    // The Log drawer was never fed. Keep the last 200 lines of user actions and errors.
    function appendLog(level, msg) {
        var next = root.logLines.slice(-199);
        next.push(Qt.formatTime(new Date(), "HH:mm:ss") + "  " + level + "  " + msg);
        root.logLines = next;
    }
    property bool showSpeedGraph: false
    property bool showLogDrawer: false

    // FQT-047: clipboard monitor
    property bool clipboardMonitorEnabled: false
    property bool ferricHostRegistered: false
    property string lastClipboardText: ""
    property string clipboardOfferUrl: ""
    property string clipboardOfferType: "" // "download" or "torrent"

    function checkClipboard() {
        if (!root.clipboardMonitorEnabled) return;
        clipboardMonitorInput.text = "";
        clipboardMonitorInput.paste();
        var clip = clipboardMonitorInput.text;
        if (clip === root.lastClipboardText || clip === "") return;
        root.lastClipboardText = clip;
        // Detect magnet
        if (root.torrentsEnabled && clip.indexOf("magnet:?xt=urn:btih:") === 0) {
            root.clipboardOfferUrl = clip;
            root.clipboardOfferType = "torrent";
            clipboardOfferToast.visible = true;
            return;
        }
        // Detect http(s) URL
        if (/^https?:\/\//i.test(clip)) {
            root.clipboardOfferUrl = clip;
            root.clipboardOfferType = "download";
            clipboardOfferToast.visible = true;
        }
    }

    function doClipboardOfferAdd() {
        if (root.clipboardOfferType === "torrent") {
            // Set the magnet in the torrent input via the dialog's public text property
            addTorrentDialog.torrentText = root.clipboardOfferUrl;
            addTorrentDialog.saveText = "";
            root.addTorrentProbeJson = ""; addTorrentDialog.probeResult = null;
            addTorrentDialog.visible = true;
        } else {
            addUrlDialog.urlText = root.clipboardOfferUrl;
            addUrlDialog.clearInputs();
            addUrlDialog.urlText = root.clipboardOfferUrl;
            addUrlDialog.visible = true;
        }
        clipboardOfferToast.visible = false;
    }

    // FQT-040: grabber
    property string grabUrl: ""
    property string grabExtensions: ""
    property int grabDepth: 0
    property string grabExclude: ""
    property string grabInclude: ""
    property bool grabStayBelow: false
    property int grabExternal: 0
    property var grabResults: []
    property var grabSelected: []
    property bool grabLoading: false

    function doGrab() {
        if (root.grabUrl.trim() === "") { showError("URL required"); return; }
        root.grabLoading = true;
        root.grabResults = [];
        root.grabSelected = [];
        requestBus("grab_links", {
            url: root.grabUrl.trim(),
            extensions: root.grabExtensions,
            depth: root.grabDepth,
            exclude: root.grabExclude,
            include: root.grabInclude,
            stay_below: root.grabStayBelow,
            external_levels: root.grabExternal
        }, function(raw) {
            root.grabLoading = false;
            try {
                var arr = JSON.parse(raw);
                if (arr.error) { showError(arr.error); return; }
                root.grabResults = arr;
                if (root.grabDownloadAll && arr.length > 0) {
                    // IDM's "Explore site and download matched files"
                    var all = [];
                    for (var k = 0; k < arr.length; k++) all.push(k);
                    root.grabSelected = all;
                    root.doGrabAddSelected();
                }
            } catch(e) { showError("Grab failed: " + e); }
        });
    }

    property var grabProjects: []
    property string grabProjectId: ""
    property bool grabDownloadAll: false

    function refreshGrabProjects() {
        requestBus("grabber_projects", {}, function(raw) {
            try { var arr = JSON.parse(raw); if (!arr.error) root.grabProjects = arr; } catch (e) { }
        });
    }

    function saveGrabProject(name, asTemplate) {
        if (String(name).trim() === "") { showError("Give the project a name"); return; }
        requestBus("grabber_project_save", {
            name: name,
            is_template: asTemplate,
            config: {
                url: root.grabUrl.trim(), extensions: root.grabExtensions, depth: root.grabDepth,
                exclude: root.grabExclude, include: root.grabInclude, stay_below: root.grabStayBelow,
                external_levels: root.grabExternal
            },
            results: asTemplate ? [] : root.grabResults
        }, function(raw) {
            try { var r = JSON.parse(raw); if (r.error) { showError(r.error); return; } } catch (e) { }
            root.refreshGrabProjects();
        });
    }

    function openGrabProject(id) {
        requestBus("grabber_project_get", { id: id }, function(raw) {
            try {
                var p = JSON.parse(raw);
                if (p.error) { showError(p.error); return; }
                grabberWorkspace.applyConfig(p.config);
                root.grabProjectId = id;
                root.grabResults = p.results || [];
                root.grabSelected = [];
            } catch (e) { showError("Could not open the project: " + e); }
        });
    }

    // IDM's "Update": explore the saved project's site again and report what changed.
    function updateGrabProject() {
        if (root.grabProjectId === "") { showError("Open a saved project first"); return; }
        root.grabLoading = true;
        requestBus("grabber_project_update", { id: root.grabProjectId }, function(raw) {
            root.grabLoading = false;
            try {
                var r = JSON.parse(raw);
                if (r.error) { showError(r.error); return; }
                showInfo(r.total + " file(s) found: " + r.added.length + " new, " + r.removed.length + " gone since the last run");
                root.openGrabProject(root.grabProjectId);
                root.refreshGrabProjects();
            } catch (e) { showError("Update failed: " + e); }
        });
    }

    function deleteGrabProject(id) {
        requestBus("grabber_project_delete", { id: id }, function(raw) { root.refreshGrabProjects(); });
    }

    function toggleGrabResult(idx) {
        var next = root.grabSelected.slice();
        var i = next.indexOf(idx);
        if (i >= 0) next.splice(i, 1); else next.push(idx);
        root.grabSelected = next;
    }

    function doGrabAddSelected() {
        var args = [];
        for (var i = 0; i < root.grabSelected.length; i++) {
            var url = root.grabResults[root.grabSelected[i]];
            if (!url) continue;
            args.push({ url: url, filename: "", save_dir: "", cookies: "", description: "" });
        }
        root.grabSelected = [];
        requestBusBatch("add_download", args, function(results) {
            var added = 0;
            for (var j = 0; j < results.length; j++)
                if (!results[j]) added++;
            downloadModel.refresh();
            if (added > 0) showError(added + " download(s) added");
        });
    }

    // FQT-041: media
    property string mediaUrl: ""
    property var mediaResult: null
    property string mediaSelectedFormat: ""
    property bool mediaLoading: false
    property string mediaError: ""

    function doMediaExtract() {
        if (root.mediaUrl.trim() === "") { root.mediaError = "URL required"; return; }
        root.mediaLoading = true;
        root.mediaResult = null;
        root.mediaSelectedFormat = "";
        root.mediaError = "";
        requestBus("extract_media", { url: root.mediaUrl.trim() }, function(raw) {
            root.mediaLoading = false;
            try {
                var res = JSON.parse(raw);
                if (res.error) { root.mediaError = res.error; return; }
                root.mediaResult = res;
                if (res.formats && res.formats.length > 0)
                    root.mediaSelectedFormat = res.formats[0].id;
            } catch(e) { root.mediaError = "Extract failed: " + e; }
        });
    }

    function doMediaDownload() {
        if (!root.mediaResult || root.mediaSelectedFormat === "") return;
        root.mediaError = "";
        requestBus("download_media", {
            url: root.mediaUrl.trim(),
            format_id: root.mediaSelectedFormat
        }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { root.mediaError = res.error; return; }
                root.mediaResult = null;
                root.mediaSelectedFormat = "";
                downloadModel.refresh();
                root.mediaError = "Download started: " + (res.title || "");
            } catch(e) { root.mediaError = "Download failed: " + e; }
        });
    }

    // FQT-042: playlist + merge jobs
    property var mergeJobs: []
    property string ffmpegStatus: ""

    function doPlaylistDownload() {
        if (root.mediaUrl.trim() === "") { root.mediaError = "URL required"; return; }
        root.mediaLoading = true;
        root.mediaError = "";
        requestBus("playlist_download", {
            url: root.mediaUrl.trim(),
            save_dir: "",
            max_items: 50
        }, function(raw) {
            root.mediaLoading = false;
            try {
                var res = JSON.parse(raw);
                if (res.error) { root.mediaError = res.error; return; }
                root.mediaError = "Playlist queued: " + res.queued + " item(s)";
                downloadModel.refresh();
            } catch(e) { root.mediaError = "Playlist failed: " + e; }
        });
    }

    function loadMergeJobs() {
        requestBus("get_merge_jobs", {}, function(raw) {
            try { root.mergeJobs = JSON.parse(raw); } catch(e) { root.mergeJobs = []; }
        });
    }

    function loadFfmpegStatus() {
        requestBus("ffmpeg_status", {}, function(raw) {
            try {
                var s = JSON.parse(raw);
                root.ffmpegStatus = s.error ? "ffmpeg ?" : s.available ? "ffmpeg \u2713" : "ffmpeg \u2717";
            } catch(e) { root.ffmpegStatus = "ffmpeg ?"; }
        });
    }

    // FQT-068: RSS
    property var rssFeeds: []
    property var rssRules: []

    function loadRssFeeds() {
        requestBus("get_rss_feeds", {}, function(raw) {
            try {
                var value = JSON.parse(raw);
                root.rssFeeds = value.error ? [] : value;
            } catch(e) { root.rssFeeds = []; }
        });
    }

    function loadRssRules() {
        requestBus("get_rss_rules", {}, function(raw) {
            try {
                var value = JSON.parse(raw);
                root.rssRules = value.error ? [] : value;
            } catch(e) { root.rssRules = []; }
        });
    }

    function doAddRssFeed(url) {
        if (url.trim() === "") { showError("Feed URL required"); return; }
        requestBus("add_rss_feed", { url: url.trim() }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { showError(res.error); return; }
                addRssDialog.visible = false;
                loadRssFeeds();
            } catch(e) { showError("Add RSS feed failed: " + e); }
        });
    }

    // FQT-043: cookies
    property string cookiesText: ""
    property string cookiesStatus: ""

    function doParseCookies() {
        if (root.cookiesText.trim() === "") { root.cookiesStatus = "Paste cookies first"; return; }
        requestBus("parse_cookies", { text: root.cookiesText }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { root.cookiesStatus = "Error: " + res.error; return; }
                root.cookiesStatus = "Parsed " + res.count + " cookie(s)" + (res.header_example ? " \u2014 header preview ready" : "");
            } catch(e) { root.cookiesStatus = "Parse failed: " + e; }
        });
    }

    function doExportCookies() {
        if (root.cookiesText.trim() === "") { root.cookiesStatus = "Paste cookies first"; return; }
        requestBus("export_cookies", { text: root.cookiesText }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { root.cookiesStatus = "Error: " + res.error; return; }
                root.cookiesText = res.text || "";
                root.cookiesStatus = "Exported " + res.count + " cookie(s) \u2014 normalized";
            } catch(e) { root.cookiesStatus = "Export failed: " + e; }
        });
    }

    // FQT-046: site rules
    property var siteRules: []
    property var editingRule: null
    property string ruleDialogMode: "add" // "add" or "edit"

    function loadSiteRules() {
        requestBus("get_site_rules", {}, function(raw) {
            try { root.siteRules = JSON.parse(raw); } catch(e) { root.siteRules = []; }
        });
    }

    function doAddSiteRule() {
        ruleDialogMode = "add";
        editingRule = null;
        siteRuleDialog.ruleDialogMode = "add";
        siteRuleDialog.editingRule = null;
        siteRuleDialog.prepareAdd();
        siteRuleDialog.visible = true;
    }

    function doEditSiteRule(rule) {
        ruleDialogMode = "edit";
        editingRule = rule;
        siteRuleDialog.ruleDialogMode = "edit";
        siteRuleDialog.editingRule = rule;
        siteRuleDialog.prepareEdit(rule);
        siteRuleDialog.visible = true;
    }

    function doDeleteSiteRule(id) {
        requestBus("delete_site_rule", { id: id }, function(err) {
            if (err) { showError(err); return; }
            loadSiteRules();
        });
    }

    function loadQueues() {
        requestBus("get_queues", {}, function(raw) {
            try {
                var value = JSON.parse(raw);
                root.queues = value.error ? [] : value;
            } catch(e) { root.queues = []; }
        });
    }

    property var editingQueue: null

    // IDM's Start Queue / Stop Queue buttons: one queue acts at once, several ask which.
    function pickQueueThen(action) {
        if (root.queues.length === 0) return;
        if (root.queues.length === 1) { root.doQueueAction(root.queues[0].id, action); return; }
        queuePickMenu.action = action;
        queuePickMenu.popup();
    }
    ThemedMenu {
        id: queuePickMenu
        property string action: "start"
        Repeater {
            model: root.queues
            ThemedMenuItem { text: modelData.name; onTriggered: root.doQueueAction(modelData.id, queuePickMenu.action) }
        }
    }

    function doEditQueue(q) {
        root.editingQueue = JSON.parse(JSON.stringify(q));
        editQueueDialog.title = qsTr("Edit Queue");
        editQueueDialog.populate(q);
        editQueueDialog.visible = true;
    }

    // IDM's Scheduler "New queue": the same editor, creating the queue first.
    function doNewQueue() {
        root.editingQueue = null;
        editQueueDialog.title = qsTr("New Queue");
        editQueueDialog.populate({ name: "", max_concurrent: 2, enabled: true, sched_days: 127 });
        editQueueDialog.visible = true;
    }
    function doQueueAction(id, action) {
        requestBus("queue_action", { id: id, action: action }, function(err) {
            if (err) showError(err);
            downloadModel.refresh();
        });
    }
    function doDeleteQueue(q) {
        requestBus("delete_queue", { id: q.id }, function(err) {
            if (err) { showError(err); return; }
            if (root.queueFilter === q.id) root.queueFilter = "";
            loadQueues();
            downloadModel.refresh();
        });
    }

    function doSaveQueue() {
        var f = editQueueDialog.readFields();
        if (!root.editingQueue) {
            if (f.name === "") { showError(qsTr("Give the queue a name.")); return; }
            requestBus("create_queue", { body_json: JSON.stringify({ name: f.name, max_concurrent: f.max_concurrent }) }, function(raw) {
                var rec;
                try { rec = JSON.parse(raw); } catch (err) { showError(raw); return; }
                if (!rec || rec.error || !rec.id) { showError((rec && rec.error) || raw); return; }
                root.editingQueue = rec;
                root.doSaveQueue();
            });
            return;
        }
        var patch = {
            name: f.name,
            max_concurrent: f.max_concurrent,
            enabled: f.enabled,
            sched_days: f.sched_days,
            start_on_startup: f.start_on_startup,
            sync_interval_min: f.sync_interval_min,
            max_retries: f.max_retries,
            open_file: f.open_file,
            when_done: f.when_done
        };
        var s = f.sched_start;
        var e = f.sched_end;
        patch.sched_start = s !== "" ? parseInt(s) : null;
        patch.sched_end = e !== "" ? parseInt(e) : null;
        requestBus("update_queue", {
            id: root.editingQueue.id,
            patch_json: JSON.stringify(patch)
        }, function(err) {
            if (err) { showError(err); return; }
            editQueueDialog.visible = false;
            loadQueues();
        });
    }

    // T0: toast list fetch failures once per distinct message (no spam every 1.5s).
    property string lastDownloadListError: ""
    property string lastTorrentListError: ""

    // Model refreshes run in the background, so the result lands *after* the call
    // that started it. Re-evaluate whenever a model publishes a new result; without
    // this a cold-start "cannot read lock file" dialog stayed on screen forever
    // even though the next refresh succeeded.
    Connections {
        target: downloadModel
        function onLast_okChanged() { root.reportListFetchErrors() }
        function onLast_errorChanged() { root.reportListFetchErrors() }
    }
    Connections {
        target: torrentModel
        function onLast_okChanged() { root.reportListFetchErrors() }
        function onLast_errorChanged() { root.reportListFetchErrors() }
    }

    // "Daemon not up (yet)" is a state, not an error worth a modal: the status bar
    // already says "Disconnected" and the models are re-fetched when it connects.
    function isTransientListError(msg) {
        return msg.indexOf("cannot read lock file") === 0
            || msg.indexOf("daemon unreachable") === 0
            || msg.indexOf("error sending request") === 0
    }

    function reportListFetchErrors() {
        var dErr = (!downloadModel.last_ok && downloadModel.last_error)
            ? ("" + downloadModel.last_error) : ""
        var tErr = (root.torrentsEnabled && !torrentModel.last_ok && torrentModel.last_error)
            ? ("" + torrentModel.last_error) : ""
        if (isTransientListError(dErr)) dErr = ""
        if (isTransientListError(tErr)) tErr = ""
        var showingListError = errorDialog.visible
            && (errorDialog.message === qsTr("Downloads list: ") + root.lastDownloadListError
                || errorDialog.message === qsTr("Torrents list: ") + root.lastTorrentListError)
        if (dErr !== "" && dErr !== root.lastDownloadListError) {
            root.lastDownloadListError = dErr
            showError(qsTr("Downloads list: ") + dErr)
        } else if (dErr === "") {
            root.lastDownloadListError = ""
        }
        if (tErr !== "" && tErr !== root.lastTorrentListError) {
            root.lastTorrentListError = tErr
            showError(qsTr("Torrents list: ") + tErr)
        } else if (tErr === "") {
            root.lastTorrentListError = ""
        }
        if (dErr === "" && tErr === "" && showingListError)
            errorDialog.close()
    }

    function showError(msg) {
        appendLog("ERROR", msg);
        errorDialog.show(msg);
    }

    function showInfo(msg) { showError(msg); }

    function formatBps(bps) {
        if (bps <= 0) return "0 B/s";
        if (bps < 1024) return bps + " B/s";
        if (bps < 1048576) return (bps / 1024).toFixed(1) + " KB/s";
        return (bps / 1048576).toFixed(1) + " MB/s";
    }

    function formatBytes(bytes) {
        if (bytes <= 0) return "0 B";
        if (bytes < 1024) return bytes + " B";
        if (bytes < 1048576) return (bytes / 1024).toFixed(1) + " KB";
        if (bytes < 1073741824) return (bytes / 1048576).toFixed(1) + " MB";
        return (bytes / 1073741824).toFixed(2) + " GB";
    }

    // FQT-081 / Phase 2: single source for anonymity score (0–100 scale)
    property var anonymityScore: null
    property string anonymityGrade: ""
    // Phase 3 residual #4: privacy dashboard (GET /privacy/dashboard via bus)
    property var privacyDashboard: ({})
    property string privacyRecommendation: ""

    function loadAnonymityScore() {
        requestBus("get_anonymity_score", {}, function(raw) {
            try {
                var s = JSON.parse(raw);
                // Torrent-only score: show N/A (null) for download-only use instead of a misleading F.
                var applicable = s.applicable !== false;
                root.anonymityScore = !applicable ? null : ((s.score !== undefined && s.score !== null) ? s.score : "?");
                root.anonymityGrade = applicable ? (s.grade || "") : "";
            } catch(e) {
                root.anonymityScore = null;
                root.anonymityGrade = "";
                console.warn("Privacy score fetch failed: " + e);
            }
        });
    }

    function loadPrivacyDashboard() {
        requestBus("get_privacy_dashboard", {}, function(raw) {
            try {
                var d = JSON.parse(raw);
                if (d && d.error) {
                    root.privacyDashboard = {};
                    root.privacyRecommendation = "";
                    return;
                }
                root.privacyDashboard = d || {};
                var recs = (d && d.recommendations) ? d.recommendations : [];
                root.privacyRecommendation = recs.length > 0 ? recs[0] : ((d && d.summary) ? d.summary : "");
            } catch(e) {
                root.privacyDashboard = {};
                root.privacyRecommendation = "";
                console.warn("Privacy dashboard fetch failed: " + e);
            }
        });
    }

    function formatAnonymityScore() {
        if (root.anonymityScore === null || root.anonymityScore === undefined)
            return qsTr("N/A");
        return root.anonymityScore + "/100";
    }

    function gradeBadgeColor() {
        switch (root.anonymityGrade) {
        case "A": return Theme.success;
        case "B": return Theme.success;
        case "C": return Theme.warning;
        case "D": return Theme.warning;
        case "F": return Theme.danger;
        default:  return Theme.textDim;
        }
    }

    function doDeleteSelected() {
        if (root.selectedDownloads.length === 0) return;
        root.doDeleteConfirm();
    }

    // ── IDM-style progress / complete windows ─────────────────────────────
    // Active downloads seen by the watcher; the first poll only seeds the set so
    // resumed downloads do not pop windows at startup.
    property var trackedDownloads: ({})
    property var progressWindows: ({})
    property bool downloadWatchSeeded: false

    Component { id: progressWindowComp; DownloadProgressWindow { requestBus: root.requestBus } }
    Component { id: completeWindowComp; DownloadCompleteWindow { requestBus: root.requestBus } }

    // ── IDM "Download File Info" dialog ───────────────────────────────────
    // Browser grabs and Add URL are held by the daemon until Start Download / Download Later /
    // Cancel is pressed in this dialog; held rows stay out of the lists meanwhile.
    property var infoDialogs: ({})
    property var pendingCaptures: ({})
    Component { id: infoDialogComp; DownloadInfoDialog { requestBus: root.requestBus } }

    function holdForDialog() { return root.prefsData["show_start_dialog"] !== "0"; }

    function setPending(id, on) {
        var p = Object.assign({}, root.pendingCaptures);
        if (on) p[id] = true; else delete p[id];
        root.pendingCaptures = p;
    }

    function openInfoDialog(id) {
        var existing = root.infoDialogs[id];
        if (existing) { existing.show(); existing.raise(); existing.requestActivate(); return; }
        root.setPending(id, true);
        var w = infoDialogComp.createObject(null, { downloadId: id, queues: root.queues, askQueue: root.prefsData["show_queue_panel"] !== "0" });
        if (!w) { root.setPending(id, false); return; }
        root.infoDialogs[id] = w;
        w.finished.connect(function(did) {
            delete root.infoDialogs[did];
            root.setPending(did, false);
            downloadModel.refresh();
            root.loadOverview();
            root.watchDownloads(); // Start Download: show the progress window now, not at the next tick
        });
        w.closing.connect(function() { w.destroy(500); });
        w.show(); w.raise(); w.requestActivate();
    }

    function openPendingCaptures() {
        requestBus("get_pending_captures", {}, function(raw) {
            var list;
            try { list = JSON.parse(raw); } catch (e) { return; }
            if (!(list instanceof Array)) return;
            for (var i = 0; i < list.length; i++) root.openInfoDialog(list[i].id);
        });
    }

    function openProgress(id) {
        var existing = root.progressWindows[id];
        if (existing) { existing.show(); existing.raise(); existing.requestActivate(); return; }
        var w = progressWindowComp.createObject(null, { downloadId: id });
        if (!w) return;
        root.progressWindows[id] = w;
        w.finished.connect(function(did, info) {
            if (root.prefsData["show_complete_dialog"] === "0") return;
            var c = completeWindowComp.createObject(null, { info: info, bus: bus });
            if (c) { c.closing.connect(function() { c.destroy(500); }); c.show(); }
            w.close();
        });
        w.closing.connect(function() { delete root.progressWindows[id]; w.destroy(500); });
        w.show();
    }

    function watchDownloads() {
        requestBus("get_downloads", {}, function(raw) {
            var list;
            try { list = JSON.parse(raw); } catch (e) { return; }
            if (!(list instanceof Array)) return;
            var open = Object.keys(root.progressWindows).length;
            for (var i = 0; i < list.length; i++) {
                var d = list[i];
                if (d.status !== "downloading" && d.status !== "connecting") continue;
                // Still behind its Download File Info dialog (early start): like IDM, the progress window
                // appears only once the user presses Start Download. Not marked as seen, so the next look opens it.
                if (root.pendingCaptures[d.id]) continue;
                if (root.trackedDownloads[d.id]) continue;
                root.trackedDownloads[d.id] = true;
                if (root.downloadWatchSeeded && root.prefsData["show_progress_dialog"] !== "0" && open < 3) {
                    root.openProgress(d.id);
                    open++;
                }
            }
            root.downloadWatchSeeded = true;
        });
    }

    Timer { interval: 1500; repeat: true; running: true; onTriggered: root.watchDownloads() }

    function selectAllDownloads() {
        requestBus("get_downloads", {}, function(raw) {
            try { root.selectedDownloads = JSON.parse(raw); }
            catch(e) { root.selectedDownloads = []; }
        });
    }

    function refreshAll() {
        downloadModel.refresh();
        torrentModel.refresh();
        session.refresh();
    }

    function runAllDownloads(operation) {
        appendLog("INFO", operation);
        requestBus(operation, {}, function(err) {
            downloadModel.refresh();
            if (err) showError(err);
        });
    }

    function promptExit() {
        showExitDialog();
    }

    function doPause() {
        appendLog("INFO", "pause " + root.selectedDownloads.length + " download(s)");
        var args = [];
        for (var i = 0; i < root.selectedDownloads.length; i++) {
            args.push({ id: root.selectedDownloads[i].id, action: "pause" });
        }
        root.selectedDownloads = [];
        requestBusBatch("download_action", args, function(results) {
            downloadModel.refresh();
            for (var j = 0; j < results.length; j++)
                if (results[j]) { showError(results[j]); break; }
        });
    }

    function doResume() {
        appendLog("INFO", "resume " + root.selectedDownloads.length + " download(s)");
        var args = [];
        for (var i = 0; i < root.selectedDownloads.length; i++) {
            args.push({ id: root.selectedDownloads[i].id, action: "resume" });
        }
        root.selectedDownloads = [];
        requestBusBatch("download_action", args, function(results) {
            downloadModel.refresh();
            for (var j = 0; j < results.length; j++)
                if (results[j]) { showError(results[j]); break; }
        });
    }

    function doStop() {
        appendLog("INFO", "stop " + root.selectedDownloads.length + " download(s)");
        var args = [];
        for (var i = 0; i < root.selectedDownloads.length; i++) {
            // IDM's Stop keeps the partial file and can be resumed; "cancel" is final.
            args.push({ id: root.selectedDownloads[i].id, action: "pause" });
        }
        root.selectedDownloads = [];
        requestBusBatch("download_action", args, function(results) {
                downloadModel.refresh();
            for (var j = 0; j < results.length; j++)
                if (results[j]) { showError(results[j]); break; }
        });
    }

    function doDelete() {
        root.pendingDeleteIds = root.selectedDownloads.map(function(s) { return s.id; });
        deleteConfirmDialog.visible = true;
    }

    function doDeleteConfirm(withFile) {
        appendLog("INFO", "delete " + root.pendingDeleteIds.length + " download(s)" + (withFile ? " and file(s)" : ""));
        var args = root.pendingDeleteIds.map(function(id) {
            return { id: id, with_file: withFile };
        });
        root.pendingDeleteIds = [];
        root.selectedDownloads = [];
        requestBusBatch("delete_download", args, function(results) {
            downloadModel.refresh();
            for (var j = 0; j < results.length; j++)
                if (results[j]) { showError(results[j]); break; }
        });
    }

    property var pendingDuplicateAdd: null

    function doAddUrl(url, filename, savePath, cookies, description) {
        appendLog("INFO", "add " + url);
        if (url.trim() === "") { showError("URL is required"); return; }
        var trimmedUrl = url.trim();
        requestBus("get_downloads", {}, function(raw) {
            try {
                var downloads = JSON.parse(raw);
                for (var i = 0; i < downloads.length; i++) {
                    if (downloads[i].url === trimmedUrl) {
                        duplicateUrlLabel.text = trimmedUrl;
                        root.pendingDuplicateAdd = { url: trimmedUrl, filename: filename.trim(), savePath: savePath.trim(), cookies: cookies.trim(), description: (description || "").trim() };
                        duplicateUrlDialog.visible = true;
                        return;
                    }
                }
            } catch(e) {}
            requestBus("add_download", {
                url: trimmedUrl,
                filename: filename.trim(),
                save_dir: savePath.trim(),
                cookies: cookies.trim(),
                description: (description || "").trim(),
                hold: root.holdForDialog()
            }, function(err) {
                addUrlDialog.visible = false;
                downloadModel.refresh();
                if (err) showError(err);
            });
        });
    }

    function doAddDuplicateConfirm() {
        var p = root.pendingDuplicateAdd;
        if (!p) { duplicateUrlDialog.visible = false; return; }
        root.pendingDuplicateAdd = null;
        duplicateUrlDialog.visible = false;
        requestBus("add_download", {
            url: p.url,
            filename: p.filename,
            save_dir: p.savePath,
            cookies: p.cookies,
            description: p.description || "",
            hold: root.holdForDialog()
        }, function(err) {
            addUrlDialog.visible = false;
            downloadModel.refresh();
            if (err) showError(err);
        });
    }

    function doAddBatch(text) {
        var lines = text.split("\n");
        var args = [];
        for (var i = 0; i < lines.length; i++) {
            var u = lines[i].trim();
            if (u === "") continue;
            args.push({ url: u, filename: "", save_dir: "", cookies: "", description: "" });
        }
        addBatchDialog.visible = false;
        requestBusBatch("add_download", args, function(results) {
            var failed = 0;
            for (var j = 0; j < results.length; j++)
                if (results[j]) failed++;
            downloadModel.refresh();
            if (failed > 0) showError(failed + " URL(s) failed (" + (results.length - failed) + " added)");
        });
    }

    // ── Torrents (FQT-031/032) ───────────────────────────────────────────
    readonly property bool torrentCanStart: root.selectedTorrents.length > 0 &&
        root.selectedTorrents.every(function(s) {
            return s.status === "Staged" || s.status === "Metadata ready" ||
                   s.status === "Ready to start" || s.status === "Error";
        })
    readonly property bool torrentCanPause: root.selectedTorrents.length > 0 &&
        root.selectedTorrents.every(function(s) { return s.status === "Downloading"; })
    readonly property bool torrentCanResume: root.selectedTorrents.length > 0 &&
        root.selectedTorrents.every(function(s) { return s.status === "Paused"; })
    readonly property bool selectionCanQueueMove: root.selectedTorrents.length === 1

    property string addTorrentProbeJson: ""
    property var pendingDeleteTorrentIds: []

    function doProbeTorrent(input) {
        if (input.trim() === "") { showError("Magnet URI or .torrent path is required"); return; }
        requestBus("probe_torrent", { input: input.trim() }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { showError(res.error); return; }
                if (!res.supported) { showError(res.explanation || "Unsupported torrent input"); return; }
                root.addTorrentProbeJson = raw;
                addTorrentDialog.probeResult = res;
            } catch(e) { showError("Probe failed: " + raw); }
        });
    }

    function doStageTorrent(saveRoot) {
        if (root.addTorrentProbeJson === "") { showError("Probe the torrent first"); return; }
        requestBus("stage_torrent", {
            probe_json: root.addTorrentProbeJson,
            save_root: saveRoot.trim()
        }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { showError(res.error); return; }
                addTorrentDialog.visible = false;
                root.addTorrentProbeJson = "";
                addTorrentDialog.probeResult = null;
                torrentModel.refresh();
                if (res.record)
                    root.selectedTorrents = [{ id: res.record.id, status: "" }];
            } catch(e) { showError("Stage failed: " + e); }
        });
    }

    function doStartTorrents() {
        var args = [];
        for (var i = 0; i < root.selectedTorrents.length; i++) {
            args.push({ id: root.selectedTorrents[i].id, action: "start" });
        }
        requestBusBatch("torrent_action", args, function(results) {
            torrentModel.refresh();
            var err = "";
            for (var j = 0; j < results.length; j++) {
                try {
                    var res = JSON.parse(results[j]);
                    if (res.error) { err = res.error; break; }
                } catch(e) { err = e.toString(); break; }
            }
            if (err === "") return;
            // Empty save folder: open Location so user can pick path instead of only a 400.
            var needFolder = (err + "").toLowerCase().indexOf("save folder") >= 0
                || (err + "").toLowerCase().indexOf("location") >= 0;
            if (needFolder && selectedTorrents.length === 1) {
                doShowSetLocation();
                showError(qsTr("Choose a download folder (Location), then Start again."));
            } else {
                showError(err);
            }
        });
    }

    function doPauseTorrents() {
        var args = [];
        for (var i = 0; i < root.selectedTorrents.length; i++) {
            args.push({ id: root.selectedTorrents[i].id, action: "pause" });
        }
        requestBusBatch("torrent_action", args, function(results) {
            torrentModel.refresh();
            for (var j = 0; j < results.length; j++) {
                try {
                    var res = JSON.parse(results[j]);
                    if (res.error) { showError(res.error); break; }
                } catch(e) { showError(e.toString()); break; }
            }
        });
    }

    function doResumeTorrents() {
        var args = [];
        for (var i = 0; i < root.selectedTorrents.length; i++) {
            args.push({ id: root.selectedTorrents[i].id, action: "resume" });
        }
        requestBusBatch("torrent_action", args, function(results) {
            torrentModel.refresh();
            for (var j = 0; j < results.length; j++) {
                try {
                    var res = JSON.parse(results[j]);
                    if (res.error) { showError(res.error); break; }
                } catch(e) { showError(e.toString()); break; }
            }
        });
    }

    // FQT-033: torrent detail panel refresh
    function refreshTorrentDetail() {
        if (selectedTorrents.length !== 1) return
        var id = selectedTorrents[0].id
        requestBus("get_torrent", { id: id }, function(raw) {
            if (selectedTorrents.length !== 1 || selectedTorrents[0].id !== id) return
            try {
                var res = JSON.parse(raw)
                torrentDetailPanel.tdData = (res && res.error) ? null : res
            } catch(e) { torrentDetailPanel.tdData = null }
        })
    }

    property var torrentPeers: []
    property string torrentPeersError: ""

    function refreshTorrentPeers() {
        if (selectedTorrents.length !== 1) return
        var id = selectedTorrents[0].id
        requestBus("get_torrent_peers", { id: id }, function(raw) {
            if (selectedTorrents.length !== 1 || selectedTorrents[0].id !== id) return
            var res
            try { res = JSON.parse(raw) }
            catch(e) { res = { error: "Unable to load peers" } }
            if (Array.isArray(res)) {
                root.torrentPeers = res
                root.torrentPeersError = ""
            } else {
                root.torrentPeers = []
                root.torrentPeersError = res.error || "Unable to load peers"
            }
        })
    }

    function doToggleTorrentFile(torrentId, file) {
        requestBus("set_torrent_file", {
            id: torrentId,
            file_id: file.id,
            selected: !file.selected,
            priority: file.priority
        }, function(raw) {
            try {
                var res = JSON.parse(raw)
                if (res.error) { showError(res.error); return; }
                torrentDetailPanel.tdData = res
            } catch(e) { showError("File update failed: " + e) }
        })
    }

    function openTrackersEdit() {
        if (selectedTorrents.length !== 1) return;
        requestBus("get_torrent", { id: selectedTorrents[0].id }, function(raw) {
            try {
                var j = JSON.parse(raw);
                var trackers = j.trackers || [];
                var text = "";
                for (var i = 0; i < trackers.length; i++)
                    if (trackers[i].url) text += trackers[i].url + "\n";
                trackersTextArea.text = text;
            } catch(e) { trackersTextArea.text = ""; }
            trackersEditDialog.visible = true;
        });
    }

    function openTorrentPolicy() {
        if (selectedTorrents.length !== 1) return;
        root.loadTorrentPolicy(selectedTorrents[0].id);
        torrentPolicyDialog.visible = true;
    }

    function closeTopDialogIn(kids) {
        if (!kids)
            return false;
        for (var i = kids.length - 1; i >= 0; i--) {
            var c = kids[i];
            if (c && c.visible === true && closeTopDialogIn(c.children))
                return true;
            if (c && c.visible === true && c.appModal === true) {
                c.visible = false;
                return true;
            }
        }
        return false;
    }

    // Closes the topmost visible AppDialog at any nesting depth.
    function closeTopDialog() {
        var kids = root.contentItem ? root.contentItem.children : root.children;
        return closeTopDialogIn(kids);
    }

    function doDeleteTorrentPrompt() {
        root.pendingDeleteTorrentIds = root.selectedTorrents.map(function(s) { return s.id; });
        deleteTorrentDialog.visible = true;
    }

    function doDeleteTorrentConfirm() {
        var args = root.pendingDeleteTorrentIds.map(function(id) {
            return { id: id, delete_files: deleteTorrentDialog.deleteFiles };
        });
        root.pendingDeleteTorrentIds = [];
        root.selectedTorrents = [];
        deleteTorrentDialog.visible = false;
        requestBusBatch("delete_torrent", args, function(results) {
            torrentModel.refresh();
            for (var j = 0; j < results.length; j++)
                if (results[j]) { showError(results[j]); break; }
        });
    }

    // FQT-037: torrent context actions
    function doCopyMagnet() {
        if (selectedTorrents.length !== 1) return;
        requestBus("get_torrent", { id: selectedTorrents[0].id }, function(raw) {
            try {
                var j = JSON.parse(raw);
                var record = j.record || j;
                var src = record.source || "";
                clipboardInput.text = src.indexOf("magnet:") === 0
                    ? src : "magnet:?xt=urn:btih:" + (record.info_hash || "");
                clipboardInput.selectAll(); clipboardInput.copy();
            } catch(e) {}
        });
    }

    function doCopyInfoHash() {
        if (selectedTorrents.length !== 1) return;
        requestBus("get_torrent", { id: selectedTorrents[0].id }, function(raw) {
            try {
                var j = JSON.parse(raw);
                var record = j.record || j;
                clipboardInput.text = record.info_hash || "";
                clipboardInput.selectAll(); clipboardInput.copy();
            } catch(e) {}
        });
    }

    function doOpenTorrentFolder() {
        if (selectedTorrents.length !== 1) return;
        requestBus("get_torrent", { id: selectedTorrents[0].id }, function(raw) {
            try {
                var j = JSON.parse(raw);
                var record = j.record || j;
                if (record.save_root) {
                    var err = bus.open_folder(record.save_root);
                    if (err) showError(err);
                }
            } catch(e) {}
        });
    }

    // FQT-063: recheck/reannounce/location
    // Engine soft-oks missing routes \u2014 surface success as info, not silence.
    function doTorrentRecheck() {
        if (selectedTorrents.length !== 1) return;
        requestBus("torrent_action", { id: selectedTorrents[0].id, action: "recheck" }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { showError(res.error); return; }
                showInfo(qsTr("Recheck requested (unsupported engines soft-succeed)"));
                refreshTorrentDetail();
            } catch(e) { showError(String(e)); }
        });
    }

    function doTorrentReannounce() {
        if (selectedTorrents.length !== 1) return;
        requestBus("torrent_action", { id: selectedTorrents[0].id, action: "reannounce" }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { showError(res.error); return; }
                showInfo(qsTr("Reannounce requested (unsupported engines soft-succeed)"));
                refreshTorrentDetail();
            } catch(e) { showError(String(e)); }
        });
    }

    property string setLocationTarget: ""

    function doShowSetLocation() {
        if (selectedTorrents.length !== 1) return;
        requestBus("get_torrent", { id: selectedTorrents[0].id }, function(raw) {
            var path = "";
            try {
                var j = JSON.parse(raw);
                path = (j.record || j).save_root || "";
            } catch(e) {}
            if (path) {
                setLocationInput.text = path;
                setLocationDialog.visible = true;
                return;
            }
            requestBus("get_settings", {}, function(settingsRaw) {
                try { path = JSON.parse(settingsRaw).default_save_dir || ""; }
                catch(e2) { path = ""; }
                setLocationInput.text = path;
                setLocationDialog.visible = true;
            });
        });
    }

    function doApplySetLocation() {
        if (selectedTorrents.length !== 1) return;
        var loc = setLocationInput.text.trim();
        if (loc === "") { showError("Location required"); return; }
        requestBus("set_torrent_location", {
            id: selectedTorrents[0].id,
            location: loc
        }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { showError(res.error); return; }
                setLocationDialog.visible = false;
                refreshTorrentDetail();
            } catch(e) { showError("Set location failed: " + e); }
        });
    }

    function showExitDialog() {
        // Ask only when something is actually running (it used to ask whenever the list was
        // non-empty, even if everything had finished).
        if ((hub.active_downloads || 0) > 0 || (root.overviewStats.active_torrents || 0) > 0) {
            exitDialog.visible = true;
        } else {
            root.quitApp();
        }
    }

    // ── Downloads column layout (masterplan §6.3) ───────────────────────
    // Speed/ETA: ProgressHub SSE cache overlaid in DownloadModel.refresh().
    // T2 honesty: Conn has no live source \u2014 default width 0 (hidden), not fake "-".
    // Reorderable freely: DownloadsTable's delegate resolves cells by role, not index.
    property var downloadColumnsAll: [
        { title: qsTr("Name"), width: 377, role: "name" },
        { title: qsTr("Size"), width: 89, role: "size" },
        { title: qsTr("Progress"), width: 144, role: "progress" },
        { title: qsTr("Status"), width: 110, role: "status" },
        { title: qsTr("Speed"), width: 89, role: "speed" },
        { title: qsTr("ETA"), width: 89, role: "eta" },
        { title: qsTr("Category"), width: 110, role: "category" },
        { title: qsTr("Added"), width: 144, role: "added" },
        { title: qsTr("Save path"), width: 220, role: "savePath" },
        { title: qsTr("Done"), width: 80, role: "done" },
        { title: qsTr("Queue"), width: 70, role: "queue" },
        { title: qsTr("Completed"), width: 120, role: "completed" },
        { title: qsTr("URL host"), width: 140, role: "urlHost" },
        { title: qsTr("Error"), width: 160, role: "error" },
        { title: qsTr("Conn"), width: 0, role: "" }
    ]
    // What a new install shows: the columns people read. The rest are one click away in Columns.
    readonly property var defaultDownloadColumns: ["Name", "Size", "Progress", "Status", "Speed", "ETA", "Category", "Added"]
    property var downloadVisibleColumns: defaultDownloadColumns
    property var downloadColumns: {
        if (downloadVisibleColumns.length === 0) return downloadColumnsAll;
        var out = [];
        for (var i = 0; i < downloadColumnsAll.length; i++) {
            var col = downloadColumnsAll[i];
            var visible = downloadVisibleColumns.indexOf(col.title) >= 0;
            out.push({ title: col.title, width: visible ? col.width : 0, role: col.role });
        }
        // Spare room goes to the Name column, so the table always fills the window.
        var used = 28 + 24 + 16; // "#", icon, scrollbar
        for (var j = 0; j < out.length; j++) used += out[j].width;
        for (var k = 0; k < out.length; k++)
            if (out[k].role === "name" && out[k].width > 0) out[k].width += Math.max(0, root.downloadViewW - used);
        return out;
    }
    property int downloadViewW: 0
    readonly property int downloadTableWidth: {
        var w = 28 + 24; // "#" + icon swatch columns
        for (var i = 0; i < root.downloadColumns.length; i++) w += root.downloadColumns[i].width;
        return w;
    }

    // ── Torrent column layout (qBittorrent-style; widths drag-resizable) ─
    // Header titles match delegate cell order (was Size/Total swapped vs Done/Size data).
    // T2 honesty: Seeds, Availability, Category have no real source \u2014 default width 0.
    property var torrentColumnsAll: [
        { title: qsTr("Name"),         width: 220 },
        { title: qsTr("Done"),         width: 80  },
        { title: qsTr("Size"),         width: 80  },
        { title: qsTr("Progress"),     width: 110 },
        { title: qsTr("Status"),       width: 100 },
        { title: qsTr("Seeds"),        width: 0   },
        { title: qsTr("Peers"),        width: 55  },
        { title: qsTr("Down Speed"),   width: 80  },
        { title: qsTr("Up Speed"),     width: 80  },
        { title: qsTr("ETA"),          width: 70  },
        { title: qsTr("Ratio"),        width: 55  },
        { title: qsTr("Category"),     width: 80  },
        { title: qsTr("Tags"),         width: 90  },
        { title: qsTr("Availability"), width: 0   },
        { title: qsTr("Uploaded"),     width: 80  },
        { title: qsTr("Save path"),    width: 160 },
        { title: qsTr("Hash"),         width: 70  },
        { title: qsTr("Added"),        width: 90  },
        { title: qsTr("Completed"),    width: 90  },
        { title: qsTr("Private"),      width: 50  }
    ]
    property var torrentVisibleColumns: []
    property var torrentColumns: {
        if (torrentVisibleColumns.length === 0) return torrentColumnsAll;
        var out = [];
        for (var i = 0; i < torrentColumnsAll.length; i++) {
            var col = torrentColumnsAll[i];
            var visible = torrentVisibleColumns.indexOf(col.title) >= 0;
            out.push({ title: col.title, width: visible ? col.width : 0 });
        }
        return out;
    }
    readonly property int torrentTableWidth: {
        var w = 28;
        for (var i = 0; i < root.torrentColumns.length; i++) w += root.torrentColumns[i].width;
        return w;
    }

    // Resize only mutates in-memory widths. Persist on mouse release (not every move).
    function setTorrentColWidth(index, width, persist) {
        // Stub columns stay at 0 \u2014 do not let drag "discover" fake Seeds/Category/Availability.
        if (index === 5 || index === 11 || index === 13)
            return;
        var minW = 40;
        var nw = Math.max(minW, Math.round(width));
        var next = [];
        var widths = [];
        for (var i = 0; i < root.torrentColumnsAll.length; i++) {
            var c = root.torrentColumnsAll[i];
            var w = (i === index) ? nw : c.width;
            next.push({ title: c.title, width: w });
            widths.push(w);
        }
        root.torrentColumnsAll = next;
        if (persist)
            root.savePref("torrent_column_widths", JSON.stringify(widths));
    }

    function applyTorrentColumnWidths(widths) {
        if (!widths || !widths.length) return;
        // Indices with no real data source \u2014 never restore a non-zero width from prefs.
        var stubIdx = { 5: true, 11: true, 13: true }; // Seeds, Category, Availability
        var next = [];
        for (var i = 0; i < root.torrentColumnsAll.length; i++) {
            var c = root.torrentColumnsAll[i];
            var w = (i < widths.length) ? Number(widths[i]) : c.width;
            if (stubIdx[i])
                w = 0;
            else if (!(w > 0))
                w = c.width;
            next.push({ title: c.title, width: w });
        }
        root.torrentColumnsAll = next;
    }

    // Prefer binding `root.torrentColumns[i].width` in QML \u2014 JS helpers are not
    // re-evaluated when the array is replaced (column resize would appear dead).

    // ── Keyboard shortcuts ───────────────────────────────────────────────
    Shortcut {
        sequence: "Ctrl+N"
        onActivated: { addUrlDialog.clearInputs(); addUrlDialog.visible = true; }
    }
    Shortcut {
        sequence: "Ctrl+Shift+N"
        onActivated: { addBatchDialog.batchText = ""; addBatchDialog.visible = true; }
    }
    Shortcut {
        sequence: "Ctrl+T"
        onActivated: {
            addTorrentDialog.torrentText = ""; addTorrentDialog.saveText = "";
            root.addTorrentProbeJson = ""; addTorrentDialog.probeResult = null;
            addTorrentDialog.visible = true;
        }
    }
    Shortcut {
        sequence: "Ctrl+P"
        onActivated: { root.loadPreferences(); prefsDialog.visible = true; }
    }
    Shortcut {
        sequence: "Ctrl+Q"
        onActivated: {
            if (root.prefsData["confirm_on_exit"] !== "0") { root.promptExit(); }
            else { root.quitApp(); }
        }
    }
    Shortcut {
        sequence: "F5"
        onActivated: { root.refreshAll(); }
    }
    Shortcut {
        sequence: "Delete"
        onActivated: { root.doDeleteSelected(); }
    }
    Shortcut {
        sequence: "Ctrl+A"
        onActivated: { root.selectAllDownloads(); }
    }
    Shortcut {
        sequence: "Escape"
        // Generic close: the old hardcoded if-chain listed 10 dialogs, so Escape was
        // dead in the other ~13 (Add URL/Torrent/Batch, Delete, Stats, …). Walking the
        // children means a new dialog can never be forgotten here again.
        onActivated: root.closeTopDialog()
    }
    Shortcut {
        sequence: "Ctrl+O"
        onActivated: {
            addTorrentDialog.torrentText = ""; addTorrentDialog.saveText = "";
            root.addTorrentProbeJson = ""; addTorrentDialog.probeResult = null;
            addTorrentDialog.visible = true;
        }
    }
    Shortcut {
        sequence: "Space"
        onActivated: {
            if (root.workspaceIndex === 1 && root.selectedDownloads.length > 0) {
                if (root.selectionCanPause) root.doPause();
                else if (root.selectionCanResume) root.doResume();
            }
        }
    }
    Shortcut {
        sequence: "Ctrl+F"
        onActivated: root.openFind()
    }
    Shortcut {
        sequence: "Ctrl+L"
        onActivated: root.runAllDownloads("clear_completed_downloads")
    }

    // ── MenuBar ─────────────────────────────────────────────────────────
    MenuBar {
        id: menuBar
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        background: Rectangle { color: Theme.surface }
        delegate: MenuBarItem {
            id: menuItem
            contentItem: Text {
                text: menuItem.text.replace("&", "")
                color: Theme.text
                font.pixelSize: Theme.fontSizeNormal
                verticalAlignment: Text.AlignVCenter
            }
            background: Rectangle {
                color: menuItem.highlighted ? Theme.surfaceAlt : Theme.surface
            }
        }

        ThemedMenu {
            title: qsTr("&File")
            Action { text: qsTr("Add &URL..."); onTriggered: addUrlDialog.visible = true }
            ThemedMenuItem {
                visible: root.torrentsEnabled
                height: visible ? implicitHeight : 0
                action: Action { text: qsTr("Add &Torrent..."); onTriggered: addTorrentDialog.visible = true }
            }
            Action { text: qsTr("Add &Batch..."); onTriggered: addBatchDialog.visible = true }
            Action { text: qsTr("Add batch from &clipboard"); onTriggered: root.openBatchFromClipboard() }
            Action {
                text: qsTr("Show &drop target")
                checkable: true
                checked: root.dropTargetShown
                onTriggered: root.setDropTarget(!root.dropTargetShown)
            }
            ThemedMenuSeparator {}
            ThemedMenu {
                title: qsTr("E&xport")
                Action { text: qsTr("To &text file"); onTriggered: root.startExport("text") }
                Action { text: qsTr("To &export file (.ef2)"); onTriggered: root.startExport("ef2") }
            }
            ThemedMenu {
                title: qsTr("&Import")
                Action { text: qsTr("From &text or export file..."); onTriggered: importDialog.open() }
            }
            ThemedMenuSeparator {}
            Action { text: qsTr("E&xit"); onTriggered: { if (root.prefsData["confirm_on_exit"] !== "0") root.promptExit(); else root.quitApp(); } }
        }
        ThemedMenu {
            title: qsTr("&Downloads")
            Action { text: qsTr("&Pause All"); onTriggered: root.runAllDownloads("pause_all_downloads") }
            Action { text: qsTr("&Resume All"); onTriggered: root.runAllDownloads("resume_all_downloads") }
            ThemedMenuSeparator {}
            Action { text: qsTr("Show &Progress Window"); enabled: root.selectedDownloads.length === 1; onTriggered: root.openProgress(root.selectedDownloads[0].id) }
            Action { text: qsTr("&Clear Completed"); onTriggered: root.runAllDownloads("clear_completed_downloads") }
            ThemedMenuSeparator {}
            Action {
                text: qsTr("Download &Now")
                enabled: root.selectedDownloads.length > 0
                onTriggered: {
                    for (var i = 0; i < root.selectedDownloads.length; i++)
                        root.requestBus("download_action", { id: root.selectedDownloads[i].id, action: "start-now" }, function() { downloadModel.refresh() });
                }
            }
            Action {
                text: qsTr("R&edownload")
                enabled: root.selectedDownloads.length > 0
                onTriggered: {
                    for (var i = 0; i < root.selectedDownloads.length; i++)
                        root.requestBus("download_action", { id: root.selectedDownloads[i].id, action: "redownload" }, function() { downloadModel.refresh() });
                }
            }
            Action {
                text: qsTr("S&cheduler...")
                enabled: root.queues.length > 0
                onTriggered: root.doEditQueue(root.queues[0])
            }
            ThemedMenu {
                title: qsTr("Start &queue")
                Repeater {
                    model: root.queues
                    ThemedMenuItem { text: modelData.name; onTriggered: root.doQueueAction(modelData.id, "start") }
                }
            }
            ThemedMenu {
                title: qsTr("Stop q&ueue")
                Repeater {
                    model: root.queues
                    ThemedMenuItem { text: modelData.name; onTriggered: root.doQueueAction(modelData.id, "stop") }
                }
            }
            Action { text: qsTr("Speed &Limiter..."); onTriggered: { root.loadPreferences(); prefsDialog.prefsTab = 3; prefsDialog.visible = true } }
            Action { text: qsTr("&Find...	Ctrl+F"); onTriggered: root.openFind() }
        }
        ThemedMenu {
            title: qsTr("&View")
            Action { text: qsTr("&Speed Graph"); checkable: true; checked: root.showSpeedGraph; onTriggered: root.showSpeedGraph = !root.showSpeedGraph }
            Action { text: qsTr("&Log Drawer"); checkable: true; checked: root.showLogDrawer; onTriggered: root.showLogDrawer = !root.showLogDrawer }
            Action { text: qsTr("&Dark Mode"); checkable: true; checked: Theme.dark; onTriggered: Theme.dark = !Theme.dark }
            ThemedMenu {
                title: qsTr("&Arrange files")
                ThemedMenuItem { text: (root.downloadSortKey === 0 ? "• " : "   ") + qsTr("By &Order Of Addition"); onTriggered: root.setDownloadSort(0) }
                ThemedMenuItem { text: (root.downloadSortKey === 1 ? "• " : "   ") + qsTr("By &File Name"); onTriggered: root.setDownloadSort(1) }
                ThemedMenuItem { text: (root.downloadSortKey === 2 ? "• " : "   ") + qsTr("By &Size"); onTriggered: root.setDownloadSort(2) }
                ThemedMenuItem { text: (root.downloadSortKey === 3 ? "• " : "   ") + qsTr("By St&atus"); onTriggered: root.setDownloadSort(3) }
                ThemedMenuItem { text: (root.downloadSortKey === 4 ? "• " : "   ") + qsTr("By &Time Left"); onTriggered: root.setDownloadSort(4) }
                ThemedMenuItem { text: (root.downloadSortKey === 5 ? "• " : "   ") + qsTr("By Transfer &Rate"); onTriggered: root.setDownloadSort(5) }
                ThemedMenuItem { text: (root.downloadSortKey === 6 ? "• " : "   ") + qsTr("By &Last Try Date"); onTriggered: root.setDownloadSort(6) }
                ThemedMenuItem { text: (root.downloadSortKey === 7 ? "• " : "   ") + qsTr("By &Description"); onTriggered: root.setDownloadSort(7) }
                ThemedMenuItem { text: (root.downloadSortKey === 8 ? "• " : "   ") + qsTr("By Save &Path"); onTriggered: root.setDownloadSort(8) }
                ThemedMenuItem { text: (root.downloadSortKey === 9 ? "• " : "   ") + qsTr("By R&eferer"); onTriggered: root.setDownloadSort(9) }
                ThemedMenuItem { text: (root.downloadSortKey === 10 ? "• " : "   ") + qsTr("By Parent &Web Page"); onTriggered: root.setDownloadSort(10) }
            }
            Action { text: qsTr("Hide &categories"); checkable: true; checked: root.hideCategories; onTriggered: root.hideCategories = !root.hideCategories }
            ThemedMenu {
                title: qsTr("&Toolbar")
                ThemedMenuItem { text: (root.toolbarSmall ? "   " : "\u2022 ") + qsTr("&Large buttons"); onTriggered: root.setPref("toolbar_small", "0") }
                ThemedMenuItem { text: (root.toolbarSmall ? "\u2022 " : "   ") + qsTr("&Small buttons"); onTriggered: root.setPref("toolbar_small", "1") }
            }
            ThemedMenu {
                title: qsTr("Tray &icon")
                ThemedMenuItem { text: (root.showTrayIcon ? "\u2022 " : "   ") + qsTr("&Show"); onTriggered: root.setPref("show_tray_icon", "1") }
                ThemedMenuItem { text: (root.showTrayIcon ? "   " : "\u2022 ") + qsTr("Do&n't show"); onTriggered: root.setPref("show_tray_icon", "0") }
            }
            ThemedMenu {
                title: qsTr("&Font")
                ThemedMenuItem { text: qsTr("&Choose font..."); onTriggered: uiFontDialog.open() }
                ThemedMenuItem { text: qsTr("&Reset to default font"); onTriggered: root.setPref("ui_font", "") }
            }
            ThemedMenuSeparator {}
            Action { text: qsTr("&Statistics..."); onTriggered: { root.loadStats(); statsDialog.visible = true } }
            Action { text: qsTr("&Keyboard Shortcuts..."); onTriggered: shortcutsDialog.visible = true }
        }
        ThemedMenu {
            title: qsTr("&Tools")
            Action { text: qsTr("&Proxy Manager..."); onTriggered: { root.loadProxies(); proxyDialog.visible = true } }
            Action { text: qsTr("&Site Rules..."); onTriggered: { root.loadSiteRules(); siteRuleDialog.visible = true } }
        }
        ThemedMenu {
            title: qsTr("&Options")
            Action { text: qsTr("&Preferences..."); onTriggered: { root.loadPreferences(); prefsDialog.visible = true } }
        }
        ThemedMenu {
            title: qsTr("&Help")
            Action { text: qsTr("&About Ferric..."); onTriggered: aboutDialog.visible = true }
        }
    }

        // ── ToolBar \u2014 extracted to ToolBarComponent.qml ──
    ToolBarComponent {
        id: toolBar
        anchors.top: menuBar.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        workspaceIndex: root.workspaceIndex
        selectedDownloads: root.selectedDownloads
        selectedTorrents: root.selectedTorrents
        canPause: root.selectionCanPause
        canResume: root.selectionCanResume
        torrentCanStart: root.torrentCanStart
        torrentCanPause: root.torrentCanPause
        torrentCanResume: root.torrentCanResume
        small: root.toolbarSmall
        onAction: function(actionId) {
            if (actionId === "dl.add") { addUrlDialog.visible = true; }
            else if (actionId === "dl.batch") { addBatchDialog.visible = true; }
            else if (actionId === "dl.pause") { root.doPause(); }
            else if (actionId === "dl.resume") { root.doResume(); }
            else if (actionId === "dl.stop") { root.doStop(); }
            else if (actionId === "dl.stopAll") root.runAllDownloads("pause_all_downloads");
            else if (actionId === "dl.delete") { root.doDelete(); }
            else if (actionId === "dl.clearCompleted") root.runAllDownloads("clear_completed_downloads");
            else if (actionId === "dl.settings") { root.loadPreferences(); prefsDialog.visible = true; }
            else if (actionId === "dl.scheduler") { if (root.queues.length > 0) root.doEditQueue(root.queues[0]); }
            else if (actionId === "dl.startQueue") { root.pickQueueThen("start"); }
            else if (actionId === "dl.stopQueue") { root.pickQueueThen("stop"); }
            else if (actionId === "torrent.add") { addTorrentDialog.visible = true; }
            else if (actionId === "torrent.start") { root.doStartTorrents(); }
            else if (actionId === "torrent.pause") { root.doPauseTorrents(); }
            else if (actionId === "torrent.resume") { root.doResumeTorrents(); }
            else if (actionId === "torrent.delete") { root.doDeleteTorrentPrompt(); }
        }
    }

    // ── Kill-switch persistent banner (T3 differentiator) ──────────────
    KillSwitchBanner {
        id: killSwitchBanner
        anchors.bottom: statusBar.top
        anchors.left: parent.left
        anchors.right: parent.right
        active: root.torrentsEnabled && root.killSwitchActive
        onResume: function() { /* wait for proxy - auto-resume handled by daemon */ }
        onDismiss: function() { root.killSwitchActive = false }
    }

    // ── Workspace tabs ───────────────────────────────────────────────────
    Rectangle {
        id: workspaceTabs
        readonly property bool compact: width < 900
        anchors.top: toolBar.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        height: compact ? 60 : 32
        color: Theme.bg

        Row {
            id: workspaceTabRow
            anchors.left: parent.left
            anchors.leftMargin: Theme.sp2
            anchors.top: parent.top
            anchors.topMargin: Theme.sp1
            spacing: Theme.sp1
            Repeater {
                model: root.workspaceNames
                Rectangle {
                    visible: modelData !== "Torrents" || root.torrentsEnabled
                    width: 90
                    height: Theme.controlH
                    radius: Theme.radius
                    color: root.workspaceIndex === index ? Theme.surfaceAlt : "transparent"
                    Text {
                        anchors.centerIn: parent
                        text: modelData
                        color: root.workspaceIndex === index ? Theme.text : Theme.textDim
                        font.pixelSize: Theme.fontSizeNormal
                    }
                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            root.workspaceIndex = index;
                            if (modelData === "Overview") root.loadOverview();
                            if (modelData === "Media") root.loadMergeJobs();
                        }
                    }
                }
            }
        }

        // Buttons that act on the download list: shown on the Downloads tab only
        Row {
            visible: root.workspaceIndex === 1
            anchors.right: parent.right
            anchors.rightMargin: Theme.sp2
            anchors.top: workspaceTabs.compact ? workspaceTabRow.bottom : parent.top
            anchors.topMargin: workspaceTabs.compact ? 4 : 4
            spacing: Theme.sp1
            Rectangle {
                property string actionId: "shell.columns"
                width: 70; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Columns"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
                MouseArea { anchors.fill: parent; onClicked: columnChooserDialog.visible = true }
            }
            Rectangle {
                property string actionId: "shell.speed_graph"
                width: 90; height: Theme.controlH; radius: Theme.radius; color: root.showSpeedGraph ? Theme.border : Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Speed Graph"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
                MouseArea { anchors.fill: parent; onClicked: root.showSpeedGraph = !root.showSpeedGraph }
            }
            Rectangle {
                property string actionId: "shell.log"
                width: 60; height: Theme.controlH; radius: Theme.radius; color: root.showLogDrawer ? Theme.border : Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Log"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
                MouseArea { anchors.fill: parent; onClicked: root.showLogDrawer = !root.showLogDrawer }
            }
        }
    }

    // ── FQT-074: Speed graph panel ────────────────────────────────────────
    Rectangle {
        id: speedGraphPanel
        visible: root.showSpeedGraph
        z: 20
        anchors { right: parent.right; top: workspaceTabs.bottom; bottom: statusBar.top }
        width: 200
        color: Theme.bg
        border.color: Theme.surfaceAlt; border.width: 1

        Column {
            anchors { fill: parent; margins: Theme.sp2 }
            spacing: Theme.sp1
            Row {
                spacing: Theme.sp1
                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Speed"); color: Theme.accent; font.pixelSize: Theme.fontSizeSmall; font.bold: true }
                Rectangle {
                    width: Theme.sp4; height: Theme.sp4; radius: Theme.radius; color: Theme.surfaceAlt
                    Text { anchors.centerIn: parent; text: qsTr("✕"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { anchors.fill: parent; onClicked: root.showSpeedGraph = false }
                }
            }
            Row {
                spacing: 1
                Repeater {
                    model: root.speedHistory
                    Rectangle {
                        width: 2
                        height: Math.max(2, (modelData / 1048576) * 80)
                        color: Theme.accent
                        anchors.bottom: parent.bottom
                    }
                }
            }
            Text {
                color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                text: "↓ " + ((hub.download_bps || 0) >= 1048576 ? ((hub.download_bps || 0) / 1048576).toFixed(1) + " MB/s" : (hub.download_bps || 0) >= 1024 ? ((hub.download_bps || 0) / 1024).toFixed(0) + " KB/s" : (hub.download_bps || 0) + " B/s")
            }
        }
    }

    // ── FQT-074: Log drawer panel ────────────────────────────────────────
    Rectangle {
        id: logDrawerPanel
        visible: root.showLogDrawer
        z: 20
        anchors { left: parent.left; right: parent.right; bottom: statusBar.top }
        height: 150
        color: Theme.bg
        border.color: Theme.surfaceAlt; border.width: 1

        Column {
            anchors { fill: parent; margins: Theme.sp2 }
            spacing: Theme.sp1
            Row {
                spacing: Theme.sp1
                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Log"); color: Theme.accent; font.pixelSize: Theme.fontSizeSmall; font.bold: true }
                Rectangle {
                    width: Theme.sp4; height: Theme.sp4; radius: Theme.radius; color: Theme.surfaceAlt
                    Text { anchors.centerIn: parent; text: qsTr("✕"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { anchors.fill: parent; onClicked: root.showLogDrawer = false }
                }
            }
            ListView {
                width: parent.width; height: parent.height - 24
                clip: true
                cacheBuffer: 200
                model: root.logLines
                onCountChanged: positionViewAtEnd()
                delegate: Text {
                    width: ListView.view.width
                    text: modelData
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; font.family: "Courier"
                    elide: Text.ElideRight
                }
            }
        }
    }

        // ── StatusBar \u2014 extracted to StatusBarComponent.qml ──
    StatusBarComponent {
        id: statusBar
        torrentsVisible: root.torrentsEnabled
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        connected: session.connected
        port: session.port
        hubLive: hub.live
        downloadBps: hub.download_bps || 0
        // S7: use overviewStats for counts (hub only exposes download_bps + active_downloads)
        uploadBps: 0
        activeDownloads: hub.active_downloads || root.overviewStats.active_downloads || 0
        totalDownloads: root.overviewStats.total_downloads || 0
        activeTorrents: root.overviewStats.active_torrents || 0
        totalTorrents: root.overviewStats.total_torrents || 0
        mode: root.prefsData["product_mode"] || "balanced"
        killSwitchActive: root.killSwitchActive
    }

    // ── Body: SideFilter + main content \u2014 extracted to SideFilter.qml + tables ──
    Row {
        anchors.top: workspaceTabs.bottom
        anchors.bottom: statusBar.top
        anchors.left: parent.left
        anchors.right: parent.right

        SideFilter {
            id: sideFilter
            torrentsVisible: root.torrentsEnabled
            visible: !root.hideCategories
            width: root.hideCategories ? 0 : Theme.sidebarW
            height: parent.height
            clip: true
            categories: root.categories
            categoryFilter: root.categoryFilter
            queues: root.queues
            queueFilter: root.queueFilter
            tags: root.tags
            tagFilter: root.tagFilter
            statusFilter: root.torrentStatusFilter
            onCategoryChanged: function(f) { root.categoryFilter = f }
            onQueueChanged: function(f) { root.queueFilter = f }
            onTagChanged: function(f) { root.tagFilter = f }
            onStatusChanged: function(f) { root.torrentStatusFilter = f }
            onEditQueue: function(q) { root.doEditQueue(q) }
            onNewQueue: function() { root.doNewQueue() }
            onQueueAction: function(id, action) { root.doQueueAction(id, action) }
            onDeleteQueue: function(q) { root.doDeleteQueue(q) }
            onAddCategory: function() { addCategoryDialog.visible = true }
        }

        Rectangle {
            width: parent.width - sideFilter.width
            height: parent.height
            color: Theme.bg

            // Workspaces
            OverviewWorkspace { torrentsVisible: root.torrentsEnabled; visible: root.workspaceIndex === 0; anchors.fill: parent; stats: root.overviewStats; anonymityScore: root.anonymityScore; anonymityGrade: root.anonymityGrade; killSwitchActive: root.killSwitchActive; privacyDashboard: root.privacyDashboard; privacyRecommendation: root.privacyRecommendation; recentTransfers: root.overviewRecent; prefsData: root.prefsData; onRefresh: function() { root.loadOverview() }; onLoadPreferences: function() { root.loadPreferences(); prefsDialog.prefsTab = 4; prefsDialog.visible = true }; onAddUrl: function(url, fn, sp, c) { root.doAddUrl(url, fn, sp, c) }; onSelectTransfer: function(id, st, n, sp, u) { root.selectedDownloads = [{ id: id, status: st, name: n, savePath: sp, urlHost: u }]; root.workspaceIndex = 1; } }
            GrabberWorkspace { id: grabberWorkspace; projects: root.grabProjects; onProjectSave: function(n, t) { root.saveGrabProject(n, t) }; onProjectOpen: function(i) { root.openGrabProject(i) }; onProjectDelete: function(i) { root.deleteGrabProject(i) }; onProjectUpdate: function() { root.updateGrabProject() }; grabDownloadAll: root.grabDownloadAll; onGrabDownloadAllChanged: root.grabDownloadAll = grabDownloadAll; visible: root.workspaceIndex === 3; anchors.fill: parent; onGrabUrlChanged: root.grabUrl = grabUrl; onGrabExtensionsChanged: root.grabExtensions = grabExtensions; onGrabDepthChanged: root.grabDepth = grabDepth; onGrabExcludeChanged: root.grabExclude = grabExclude; onGrabIncludeChanged: root.grabInclude = grabInclude; onGrabStayBelowChanged: root.grabStayBelow = grabStayBelow; onGrabExternalChanged: root.grabExternal = grabExternal; grabUrl: root.grabUrl; grabExtensions: root.grabExtensions; grabDepth: root.grabDepth; grabResults: root.grabResults; grabSelected: root.grabSelected; grabLoading: root.grabLoading; onGrab: function() { root.doGrab() }; onToggleResult: function(i) { root.toggleGrabResult(i) }; onAddSelected: function() { root.doGrabAddSelected() } }
            MediaWorkspace { visible: root.workspaceIndex === 4; anchors.fill: parent; mediaUrl: root.mediaUrl; mediaResult: root.mediaResult; mediaSelectedFormat: root.mediaSelectedFormat; mediaLoading: root.mediaLoading; mediaError: root.mediaError; mergeJobs: root.mergeJobs; ffmpegStatus: root.ffmpegStatus; cookiesText: root.cookiesText; cookiesStatus: root.cookiesStatus; bus: bus; requestBus: root.requestBus; onExtract: function() { root.doMediaExtract() }; onPlaylistDownload: function() { root.doPlaylistDownload() }; onMediaDownload: function() { root.doMediaDownload() }; onParseCookies: function() { root.doParseCookies() }; onExportCookies: function() { root.doExportCookies() }; onLoadMergeJobs: function() { root.loadMergeJobs() }; onLoadFfmpegStatus: function() { root.loadFfmpegStatus() }; onSetMediaUrl: function(u) { root.mediaUrl = u }; onSetCookiesText: function(t) { root.cookiesText = t }; onSetMediaSelectedFormat: function(i) { root.mediaSelectedFormat = i } }
            RSSWorkspace {
                visible: root.workspaceIndex === 5
                anchors.fill: parent
                rssFeeds: root.rssFeeds
                rssRules: root.rssRules
                bus: bus
                onAddFeed: function() { addRssDialog.visible = true }
                onRefreshFeeds: function() { root.loadRssFeeds(); root.loadRssRules() }
                onDeleteFeed: function(id) {
                    requestBus("delete_rss_feed", { id: id }, function(err) {
                        if (err) { showError(err); return }
                        root.loadRssFeeds()
                        root.loadRssRules()
                    })
                }
                onAddRule: function() {
                    if (root.rssFeeds.length === 0) {
                        showError("Add an RSS feed first")
                        return
                    }
                    rssRuleFeedModel.clear()
                    for (var i = 0; i < root.rssFeeds.length; i++)
                        rssRuleFeedModel.append({ text: root.rssFeeds[i].url, feedId: root.rssFeeds[i].id })
                    addRssRuleDialog.visible = true
                }
                onDeleteRule: function(id) {
                    requestBus("delete_rss_rule", { id: id }, function(err) {
                        if (err) showError(err)
                        root.loadRssRules()
                    })
                }
                showError: root.showError
            }

            // Tables
            DownloadsTable { visible: root.workspaceIndex === 1; anchors.fill: parent; onWidthChanged: root.downloadViewW = width; downloadModel: downloadModel; columns: root.downloadColumns; tableWidth: root.downloadTableWidth; selectedDownloads: root.selectedDownloads; categoryFilter: root.categoryFilter; queueFilter: root.queueFilter; queues: root.queues; findText: root.findText; findScope: root.findScope; findMatchCase: root.findMatchCase; findWhole: root.findWhole; hiddenIds: root.pendingCaptures; onOpenProgress: function(id) { root.openProgress(id) }; onToggleSelection: function(id, st, n, sp, u, add) { root.toggleDownloadSelection(id, st, n, sp, u, add) }; onShowProperties: function(id, st, n, sp, u) { root.toggleDownloadSelection(id, st, n, sp, u, false) }; bus: bus; requestBus: root.requestBus; lastError: downloadModel.last_error; lastOk: downloadModel.last_ok }
            TorrentsTable { visible: root.workspaceIndex === 2; anchors.fill: parent; torrentModel: torrentModel; columns: root.torrentColumns; columnsAll: root.torrentColumnsAll; tableWidth: root.torrentTableWidth; selectedTorrents: root.selectedTorrents; tagFilter: root.tagFilter; statusFilter: root.torrentStatusFilter; onToggleSelection: function(id, st, add) { root.toggleTorrentSelection(id, st, add) }; onSetColWidth: function(i, w, f) { root.setTorrentColWidth(i, w, f) }; onDeleteTorrent: function(id) { root.pendingDeleteTorrentIds = [id]; deleteTorrentDialog.visible = true }; onSetLocation: function(id) { root.setLocationTarget = id; root.doShowSetLocation() }; bus: bus; requestBus: root.requestBus; lastError: torrentModel.last_error; lastOk: torrentModel.last_ok }
        }
    }

    // ── Delete confirm dialog (FQT-021) ──────────────────────────────────
    // FQT-027: clipboard helper
    TextInput { id: clipboardInput; visible: false; width: 0; height: 0 }

    // IDM's scheduler "when done": exit or turn the computer off once nothing is left to download.
    // Active count can dip to 0 between two queued starts, so the list is re-read before acting.
    // Per-queue options (IDM keeps "open this file" and the completion actions per queue); the
    // global preferences apply once every download is done.
    function anyWhenDoneConfigured() {
        if ((root.prefsData["when_done_action"] || "none") !== "none" || (root.prefsData["open_file_when_done"] || "") !== "") return true;
        return root.queues.some(function(q) { return !!q.when_done || !!q.open_file; });
    }
    function openLocalFile(path) { Qt.openUrlExternally("file:///" + path.split("\\").join("/")); }
    property var queueFiredAt: ({})
    readonly property double startedAtMs: Date.now()
    Timer {
        id: whenDoneCheck
        interval: 8000
        onTriggered: {
            if (!root.anyWhenDoneConfigured()) return;
            root.requestBus("get_downloads", {}, function(raw) {
                var rows;
                try { rows = JSON.parse(raw); } catch (e) { return; }
                if (!Array.isArray(rows)) return;
                function isBusy(r) { return r.status === "downloading" || r.status === "connecting" || r.status === "queued"; }
                var defaultQueue = "00000000-0000-0000-0000-000000000001";
                var pendingAction = "";
                root.queues.forEach(function(q) {
                    if (!q.when_done && !q.open_file) return;
                    var mine = rows.filter(function(r) { return (r.queue_id || defaultQueue) === q.id; });
                    if (mine.length === 0 || mine.some(isBusy)) return;
                    var latest = 0;
                    mine.forEach(function(r) { if (r.completed_at && r.completed_at > latest) latest = r.completed_at; });
                    var seen = root.queueFiredAt[q.id];
                    if (seen === undefined) seen = root.startedAtMs;  // only react to downloads finished since Ferric started
                    if (latest <= seen) return;
                    root.queueFiredAt[q.id] = latest;
                    if (q.open_file) root.openLocalFile(q.open_file);
                    if (q.when_done) pendingAction = q.when_done;
                });
                if (!rows.some(isBusy)) {
                    var openFile = root.prefsData["open_file_when_done"] || "";
                    if (openFile !== "") root.openLocalFile(openFile);
                    var action = root.prefsData["when_done_action"] || "none";
                    if (action !== "none") pendingAction = action;
                }
                if (pendingAction === "exit") root.quitApp();
                else if (pendingAction === "shutdown") root.beginShutdownCountdown();
            });
        }
    }
    property int shutdownSecondsLeft: 60
    function beginShutdownCountdown() {
        var err = bus.system_power("shutdown");
        if (err) { root.showError(qsTr("Could not start the shutdown: ") + err); return; }
        root.shutdownSecondsLeft = 60;
        shutdownDialog.visible = true;
        shutdownTick.start();
    }
    function cancelShutdown() {
        shutdownTick.stop();
        bus.system_power("abort");
        shutdownDialog.visible = false;
    }
    Timer {
        id: shutdownTick
        interval: 1000
        repeat: true
        onTriggered: { if (root.shutdownSecondsLeft > 0) root.shutdownSecondsLeft--; }
    }
    AppDialog {
        id: shutdownDialog
        title: qsTr("Turning off the computer")
        contentItem: [
            Text {
                width: parent.width; wrapMode: Text.WordWrap
                color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                text: qsTr("All downloads are finished. The computer turns off in %1 seconds.").arg(root.shutdownSecondsLeft)
            }
        ]
        buttons: [
            Rectangle {
                width: Theme.btnL; height: Theme.controlH; radius: Theme.radius; color: Theme.accent
                Text { anchors.centerIn: parent; text: qsTr("Cancel shutdown"); color: "white"; font.pixelSize: Theme.fontSizeNormal }
                MouseArea { anchors.fill: parent; onClicked: root.cancelShutdown() }
            }
        ]
    }

    // IDM's Tasks > Export / Import of the download list (text file or IDM export file).
    function localPath(fileUrl) {
        return decodeURIComponent(fileUrl.toString().replace(/^file:\/\/\/([A-Za-z]:)/, "$1").replace(/^file:\/\//, ""));
    }
    function startExport(fmt) {
        exportDialog.fmt = fmt;
        exportDialog.nameFilters = fmt === "ef2" ? [qsTr("Export files (*.ef2)")] : [qsTr("Text files (*.txt)")];
        exportDialog.defaultSuffix = fmt === "ef2" ? "ef2" : "txt";
        exportDialog.open();
    }
    function finishExport(fileUrl) {
        // The selected downloads when there are any, otherwise the whole list.
        var ids = JSON.stringify(root.selectedDownloads.map(function(d) { return d.id; }));
        if (root.selectedDownloads.length === 0) ids = "[]";
        var content = bus.build_list_export(exportDialog.fmt, ids);
        if (content.indexOf("{\"error\"") === 0) { root.showError(qsTr("Export failed: ") + content); return; }
        var err = bus.write_text_file(root.localPath(fileUrl), content);
        if (err) root.showError(qsTr("Could not write the file: ") + err);
    }
    function finishImport(fileUrl) {
        var urls = bus.read_list_file(root.localPath(fileUrl));
        if (urls.indexOf("{\"error\"") === 0) { root.showError(qsTr("Import failed: ") + urls); return; }
        if (urls === "") { root.showError(qsTr("No download links were found in that file.")); return; }
        addBatchDialog.clearInputs();
        addBatchDialog.batchText = urls;
        addBatchDialog.visible = true;
    }
    Platform.FileDialog {
        id: exportDialog
        property string fmt: "text"
        title: qsTr("Export download list")
        fileMode: Platform.FileDialog.SaveFile
        onAccepted: root.finishExport(file)
    }
    Platform.FileDialog {
        id: importDialog
        title: qsTr("Import download list")
        fileMode: Platform.FileDialog.OpenFile
        nameFilters: [qsTr("Download lists (*.txt *.ef2)"), qsTr("All files (*)")]
        onAccepted: root.finishImport(file)
    }

    // IDM's "Add batch download from clipboard": every URL line on the clipboard, ready to confirm.
    function openBatchFromClipboard() {
        clipboardInput.text = "";
        clipboardInput.paste();
        addBatchDialog.clearInputs();
        addBatchDialog.batchText = clipboardInput.text;
        addBatchDialog.visible = true;
    }

    // IDM's Find (Ctrl-F) for the download list.
    property string findText: ""
    property string findScope: "name"
    property bool findMatchCase: false
    property bool findWhole: false
    property bool findOpen: false
    function openFind() {
        root.workspaceIndex = 1;
        root.findOpen = true;
        findInput.forceActiveFocus();
        findInput.selectAll();
    }
    function closeFind() {
        root.findOpen = false;
        root.findText = "";
    }
    Rectangle {
        id: findBar
        visible: root.findOpen && root.workspaceIndex === 1
        z: 20
        width: 520; height: Theme.controlH; radius: Theme.radius
        anchors { right: parent.right; bottom: parent.bottom; rightMargin: Theme.sp4; bottomMargin: Theme.sp5 }
        color: Theme.surface; border.color: findInput.activeFocus ? Theme.accent : Theme.surfaceAlt; border.width: 1
        Text { anchors.verticalCenter: parent.verticalCenter; id: findLabel; text: qsTr("Find"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors { left: parent.left; leftMargin: Theme.sp2; verticalCenter: parent.verticalCenter } }
        TextInput {
            id: findInput
            anchors { left: findLabel.right; leftMargin: Theme.sp2; right: findOptions.left; rightMargin: Theme.sp1; verticalCenter: parent.verticalCenter }
            color: Theme.text; font.pixelSize: Theme.fontSizeNormal; clip: true
            text: root.findText
            onTextChanged: root.findText = text
            Keys.onEscapePressed: root.closeFind()
        }
        Row {
            id: findOptions
            spacing: Theme.sp1
            anchors { right: findClose.left; rightMargin: Theme.sp2; verticalCenter: parent.verticalCenter }
            Repeater {
                model: [
                    { kind: "scope", id: "name", label: qsTr("File name") },
                    { kind: "scope", id: "description", label: qsTr("Description") },
                    { kind: "scope", id: "site", label: qsTr("Site / folder") },
                    { kind: "case", id: "case", label: qsTr("Match case") },
                    { kind: "whole", id: "whole", label: qsTr("Whole string") }
                ]
                Rectangle {
                    property bool on: modelData.kind === "scope" ? root.findScope === modelData.id
                                    : modelData.kind === "case" ? root.findMatchCase : root.findWhole
                    width: optLabel.implicitWidth + 12; height: Theme.rowH; radius: Theme.radius
                    color: on ? Theme.accent : Theme.surfaceAlt
                    Text { id: optLabel; anchors.centerIn: parent; text: modelData.label; font.pixelSize: Theme.fontSizeSmall; color: parent.on ? "white" : Theme.text }
                    MouseArea { anchors.fill: parent; onClicked: {
                        if (modelData.kind === "scope") root.findScope = modelData.id
                        else if (modelData.kind === "case") root.findMatchCase = !root.findMatchCase
                        else root.findWhole = !root.findWhole
                    } }
                }
            }
        }
        Text {
            id: findClose
            text: "✕"; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal
            anchors { right: parent.right; rightMargin: Theme.sp2; verticalCenter: parent.verticalCenter }
            MouseArea { anchors.fill: parent; anchors.margins: -4; onClicked: root.closeFind() }
        }
    }

    // FQT-047: clipboard monitor
    TextInput { id: clipboardMonitorInput; visible: false; width: 0; height: 0 }

    Timer {
        id: clipboardTimer
        interval: 2000
        running: root.clipboardMonitorEnabled
        repeat: true
        onTriggered: root.checkClipboard()
    }

    // FQT-027
    function doOpenFolder() {
        if (selectedDownloads.length !== 1) return
        var err = bus.open_folder(selectedDownloads[0].savePath)
        if (err) showError(err)
    }
    function doOpenFile() {
        if (selectedDownloads.length !== 1) return
        var err = bus.open_file(selectedDownloads[0].savePath + "/" + selectedDownloads[0].name)
        if (err) showError(err)
    }
    function doCopyUrl() {
        if (selectedDownloads.length !== 1) return
        var selected = selectedDownloads[0]
        requestBus("get_download", { id: selected.id }, function(raw) {
            try {
                var j = JSON.parse(raw)
                clipboardInput.text = j.url || selected.urlHost
            } catch(e) { clipboardInput.text = selected.urlHost }
            clipboardInput.selectAll()
            clipboardInput.copy()
        })
    }
    function doCopyName() {
        if (selectedDownloads.length !== 1) return
        clipboardInput.text = selectedDownloads[0].name
        clipboardInput.selectAll()
        clipboardInput.copy()
    }

    // FQT-036: per-torrent policy
    property var torrentPolicyData: null
    property string torrentPolicyId: ""

    function loadTorrentPolicy(id) {
        root.torrentPolicyId = id;
        requestBus("get_torrent", { id: id }, function(raw) {
            if (root.torrentPolicyId !== id) return;
            try {
                var detail = JSON.parse(raw);
                root.torrentPolicyData = detail.record ? detail.record.policy : detail.policy || {};
            } catch(e) { root.torrentPolicyData = {}; }
        });
    }

    // ponytail: reassigns the whole object (not just mutates a nested field)
    // so the `checked:` bindings below actually re-evaluate \u2014 QML's property-var
    // change notification only fires on whole-value reassignment, not on
    // mutating a field of the existing object in place. Confirmed against
    // Qt's own documented `property var` semantics; every checkbox in this
    // dialog shared this bug before the fix.
    function togglePolicyField(field) {
        var p = Object.assign({}, root.torrentPolicyData);
        p[field] = !p[field];
        root.torrentPolicyData = p;
    }

    function saveTorrentPolicy() {
        if (!root.torrentPolicyData || root.torrentPolicyId === "") return;
        var p = root.torrentPolicyData;
        var body = JSON.stringify({
            dht_enabled: !!p.dht_enabled,
            pex_enabled: !!p.pex_enabled,
            lsd_enabled: !!p.lsd_enabled,
            seed_enabled: !!p.seed_enabled,
            seed_ratio_limit: p.seed_ratio_limit != null ? p.seed_ratio_limit : null,
            seed_time_limit_min: p.seed_time_limit_min != null ? p.seed_time_limit_min : null,
            upload_limit_bps: p.upload_limit_bps != null ? p.upload_limit_bps : null,
            proxy_required: !!p.proxy_required,
            blocklist_enabled: !!p.blocklist_enabled,
            mse_required: !!p.mse_required
        });
        requestBus("update_torrent_policy", {
            id: root.torrentPolicyId,
            policy_json: body
        }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { showError(res.error); return; }
                torrentPolicyDialog.visible = false;
                refreshTorrentDetail();
            } catch(e) { showError("Policy update failed: " + e); }
        });
    }

    function doPrivacyCheck() {
        if (selectedTorrents.length !== 1) return;
        requestBus("privacy_check_torrent", { id: selectedTorrents[0].id }, function(raw) {
            try {
                var res = JSON.parse(raw);
                if (res.error) { showError(res.error); return; }
                privacyCheckResult.passed = res.passed;
                privacyCheckResult.violations = res.violations || [];
                privacyCheckResult.warnings = res.warnings || [];
                privacyCheckResult.visible = true;
                privacyToastTimer.restart();
            } catch(e) { showError("Privacy check failed: " + e); }
        });
    }

    // FQT-028
    function doShowSpeedLimit() {
        speedLimitInput.text = "0"
        speedLimitDialog.visible = true
        speedLimitInput.forceActiveFocus()
    }
    function doApplySpeedLimit() {
        var bps = (parseInt(speedLimitInput.text) || 0) * 1024
        var operation = selectedDownloads.length === 1
            ? "set_download_speed_limit" : "set_global_speed_limit"
        var args = selectedDownloads.length === 1
            ? { id: selectedDownloads[0].id, bps: bps } : { bps: bps }
        requestBus(operation, args, function(err) {
            if (err) { showError(err); return }
            speedLimitDialog.visible = false
        })
    }

    // Server asked for credentials (HTTP 401): put them into the URL and retry, as IDM's login box does.
    property string loginDownloadId: ""
    property bool loginIsProxy: false
    function doApplyLogin() {
        var id = loginDownloadId
        if (root.loginIsProxy) {
            // The proxy asked for a login (HTTP 407): put it into this download's proxy address and retry.
            requestBus("get_download", { id: id }, function(raw) {
                try {
                    var d = JSON.parse(raw)
                    var base = d.proxy_url || root.prefsData["proxy_url"] || ""
                    if (!base) { showError(qsTr("This download goes through a proxy that Ferric does not know the address of (system proxy).")); loginDialog.visible = false; return }
                    var p = new URL(base)
                    p.username = loginUser.text
                    p.password = loginPass.text
                    requestBus("refresh_download_url", { id: id, url: d.url, proxy_url: p.toString() }, function(err) { if (err) showError(err) })
                } catch(e) { showError("Proxy login failed: " + e) }
                loginDialog.visible = false
            })
            return
        }
        requestBus("get_download", { id: id }, function(raw) {
            try {
                var u = new URL(JSON.parse(raw).url)
                if (loginSave.checked)
                    requestBus("save_site_login", { body_json: JSON.stringify({ server: u.host, username: loginUser.text, password: loginPass.text }) }, function() {})
                u.username = loginUser.text
                u.password = loginPass.text
                requestBus("refresh_download_url", { id: id, url: u.toString() }, function(err) {
                    if (err) showError(err)
                })
            } catch(e) { showError("Login failed: " + e) }
            loginDialog.visible = false
        })
    }
    // FQT-029
    function doShowRefreshUrl() {
        if (selectedDownloads.length !== 1) return
        refreshUrlInput.text = ""
        refreshUrlDialog.visible = true
        refreshUrlInput.forceActiveFocus()
    }
    function doApplyRefreshUrl() {
        requestBus("refresh_download_url", {
            id: selectedDownloads[0].id,
            url: refreshUrlInput.text.trim()
        }, function(err) {
            if (err) { showError(err); return }
            refreshUrlDialog.visible = false
        })
    }
    function doPriorityUp() {
        if (selectedDownloads.length !== 1) return
        requestBus("set_download_priority", { id: selectedDownloads[0].id, priority: 1 }, function(err) {
            if (err) showError(err)
        })
    }
    function doPriorityDown() {
        if (selectedDownloads.length !== 1) return
        requestBus("set_download_priority", { id: selectedDownloads[0].id, priority: -1 }, function(err) {
            if (err) showError(err)
        })
    }

    // FQT-025: download detail panel - extracted to DownloadDetailPanel.qml
    DownloadDetailPanel {
        id: detailPanel
        visible: selectedDownloads.length === 1 && root.workspaceIndex === 1
        anchors { left: parent.left; right: parent.right; bottom: parent.bottom; bottomMargin: Theme.sp5 }

        bus: bus
        requestBus: root.requestBus
        hub: hub
        selectedDownloads: root.selectedDownloads

        doOpenFolder: root.doOpenFolder
        doOpenFile: root.doOpenFile
        doCopyUrl: root.doCopyUrl
        doCopyName: root.doCopyName
        doShowRefreshUrl: root.doShowRefreshUrl
        doPriorityUp: root.doPriorityUp
        doPriorityDown: root.doPriorityDown
        doShowSpeedLimit: root.doShowSpeedLimit
    }

    // FQT-033: torrent detail panel \u2014 extracted to TorrentDetailPanel.qml (Phase 2 split #2)
    TorrentDetailPanel {
        id: torrentDetailPanel
        visible: selectedTorrents.length === 1 && root.workspaceIndex === 2
        anchors { left: parent.left; right: parent.right; bottom: parent.bottom; bottomMargin: Theme.sp5 }

        bus: bus
        requestBus: root.requestBus
        hub: hub
        selectedTorrents: root.selectedTorrents
        selectionCanQueueMove: root.selectionCanQueueMove
        torrentPeers: root.torrentPeers
        torrentPeersError: root.torrentPeersError

        refreshTorrentDetail: root.refreshTorrentDetail
        refreshTorrentPeers: root.refreshTorrentPeers
        doToggleTorrentFile: root.doToggleTorrentFile
        doOpenTorrentFolder: root.doOpenTorrentFolder
        doCopyInfoHash: root.doCopyInfoHash
        doCopyMagnet: root.doCopyMagnet
        doTorrentRecheck: root.doTorrentRecheck
        doTorrentReannounce: root.doTorrentReannounce
        doShowSetLocation: root.doShowSetLocation
        openTrackersEdit: root.openTrackersEdit
        openTorrentPolicy: root.openTorrentPolicy
        showError: root.showError
    }


    // FQT-028: speed limit dialog
    Rectangle {
        id: speedLimitDialog
        visible: false; z: 100
        x: 0; y: 0
        width: root.width; height: root.height
        color: "#CC000000"
        Rectangle {
            width: 320; height: 168
            anchors.centerIn: parent
            color: Theme.surface; radius: Theme.radius
            Column {
                anchors { fill: parent; margins: Theme.sp4 }
                spacing: Theme.sp3
                Text {
                    text: selectedDownloads.length === 1 ? "Speed limit (KB/s) \u2014 " + selectedDownloads[0].name : "Global speed limit (KB/s)"
                    color: Theme.text; font.pixelSize: Theme.fontSizeNormal; wrapMode: Text.Wrap; width: parent.width
                }
                Text { text: qsTr("0 = unlimited"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: parent.width; height: Theme.controlH; color: Theme.bg
                    border.color: speedLimitInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                    TextInput {
                        clip: true
                        id: speedLimitInput
                        anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                        inputMethodHints: Qt.ImhDigitsOnly
                        Keys.onReturnPressed: doApplySpeedLimit()
                        Keys.onEscapePressed: speedLimitDialog.visible = false
                    }
                }
                Row {
                    spacing: Theme.sp2
                    Rectangle {
                        width: 70; height: Theme.controlH; radius: Theme.radius
                        color: slApplyArea.containsMouse ? Theme.accentDark : Theme.accent
                        Text { anchors.centerIn: parent; text: qsTr("Apply"); color: "white"; font.pixelSize: Theme.fontSizeNormal }
                        MouseArea { id: slApplyArea; anchors.fill: parent; hoverEnabled: true; onClicked: doApplySpeedLimit() }
                    }
                    Rectangle {
                        width: 70; height: Theme.controlH; radius: Theme.radius
                        color: slCancelArea.containsMouse ? Theme.border : Theme.surfaceAlt
                        Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
                        MouseArea { id: slCancelArea; anchors.fill: parent; hoverEnabled: true; onClicked: speedLimitDialog.visible = false }
                    }
                }
            }
        }
    }

    Rectangle {
        id: loginDialog
        visible: false; z: 100
        width: root.width; height: root.height
        color: "#CC000000"
        Rectangle {
            width: 340; height: 266
            anchors.centerIn: parent
            color: Theme.surface; radius: Theme.radius
            Column {
                anchors { fill: parent; margins: Theme.sp4 }
                spacing: Theme.sp2
                Text { text: root.loginIsProxy ? qsTr("The proxy asks for a user name and password.") : qsTr("Type your user name and password."); color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
                Text { text: qsTr("User name"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: parent.width; height: Theme.controlH; color: Theme.bg; radius: Theme.radius
                    border.color: loginUser.activeFocus ? Theme.accent : Theme.surfaceAlt
                    TextInput { id: loginUser; clip: true; anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                        KeyNavigation.tab: loginPass
                        Keys.onEscapePressed: loginDialog.visible = false }
                }
                Text { text: qsTr("Password"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: parent.width; height: Theme.controlH; color: Theme.bg; radius: Theme.radius
                    border.color: loginPass.activeFocus ? Theme.accent : Theme.surfaceAlt
                    TextInput { id: loginPass; clip: true; echoMode: TextInput.Password; anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                        Keys.onReturnPressed: doApplyLogin()
                        Keys.onEscapePressed: loginDialog.visible = false }
                }
                Row { spacing: Theme.sp2; height: root.loginIsProxy ? 0 : 22; visible: !root.loginIsProxy
                    Rectangle { id: loginSave; property bool checked: false; width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                        color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.checked ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: parent.checked = !parent.checked } }
                    Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Save this password in the site logins list"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                }
                Row {
                    spacing: Theme.sp2
                    Rectangle {
                        width: 70; height: Theme.controlH; radius: Theme.radius; color: Theme.accentDark
                        Text { anchors.centerIn: parent; text: qsTr("OK"); color: "white"; font.pixelSize: Theme.fontSizeNormal }
                        MouseArea { anchors.fill: parent; onClicked: doApplyLogin() }
                    }
                    Rectangle {
                        width: 70; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
                        Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
                        MouseArea { anchors.fill: parent; onClicked: loginDialog.visible = false }
                    }
                }
            }
        }
    }

    // FQT-029: refresh URL dialog
    Rectangle {
        id: refreshUrlDialog
        visible: false; z: 100
        x: 0; y: 0
        width: root.width; height: root.height
        color: "#CC000000"
        Rectangle {
            width: 440; height: 148
            anchors.centerIn: parent
            color: Theme.surface; radius: Theme.radius
            Column {
                anchors { fill: parent; margins: Theme.sp4 }
                spacing: Theme.sp3
                Text { text: qsTr("Refresh download URL"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
                Rectangle {
                    width: parent.width; height: Theme.controlH; color: Theme.bg
                    border.color: refreshUrlInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                    TextInput {
                        clip: true
                        id: refreshUrlInput
                        anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                        Keys.onReturnPressed: doApplyRefreshUrl()
                        Keys.onEscapePressed: refreshUrlDialog.visible = false
                    }
                }
                Row {
                    spacing: Theme.sp2
                    Rectangle {
                        width: 70; height: Theme.controlH; radius: Theme.radius
                        color: ruApplyArea.containsMouse ? Theme.accentDark : Theme.accent
                        Text { anchors.centerIn: parent; text: qsTr("Apply"); color: "white"; font.pixelSize: Theme.fontSizeNormal }
                        MouseArea { id: ruApplyArea; anchors.fill: parent; hoverEnabled: true; onClicked: doApplyRefreshUrl() }
                    }
                    Rectangle {
                        width: 70; height: Theme.controlH; radius: Theme.radius
                        color: ruCancelArea.containsMouse ? Theme.border : Theme.surfaceAlt
                        Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
                        MouseArea { id: ruCancelArea; anchors.fill: parent; hoverEnabled: true; onClicked: refreshUrlDialog.visible = false }
                    }
                }
            }
        }
    }

    Rectangle {
        id: deleteConfirmDialog
        visible: false
        x: 0; y: 0
        width: root.width; height: root.height
        color: "#CC000000"
        z: 100

        Rectangle {
            anchors.centerIn: parent
            width: 340; height: 150
            radius: Theme.radius
            color: Theme.surface

            Column {
                anchors.centerIn: parent
                spacing: Theme.sp4

                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "Delete " + root.pendingDeleteIds.length + " download(s)?"
                    color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                }

                Row {
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: Theme.sp2

                    Rectangle {
                        width: 110; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
                        Text { anchors.centerIn: parent; text: qsTr("Record only"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: { root.doDeleteConfirm(false); deleteConfirmDialog.visible = false; }
                        }
                    }
                    Rectangle {
                        width: 120; height: Theme.controlH; radius: Theme.radius; color: Theme.danger
                        Text { anchors.centerIn: parent; text: qsTr("Record + File"); color: "#ffffff"; font.pixelSize: Theme.fontSizeNormal }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: { root.doDeleteConfirm(true); deleteConfirmDialog.visible = false; }
                        }
                    }
                    Rectangle {
                        width: 64; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
                        Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: { root.pendingDeleteIds = []; deleteConfirmDialog.visible = false; }
                        }
                    }
                }
            }
        }
    }

    // Error dialog (FQT-021) \u2014 extracted to ErrorDialog.qml
    ErrorDialog {
        id: errorDialog
        // Above every other modal: a result/error raised while e.g. the Proxy Manager
        // is open (Test button) used to appear *behind* it and only show after closing.
        z: 1000
    }

    // ── Add URL dialog (FQT-022) \u2014 extracted to AddUrlDialog.qml ──
    AddUrlDialog {
        id: addUrlDialog
        rootWidth: root.width
        rootHeight: root.height
        skipInfoStep: root.holdForDialog()
        onAdd: function(url, filename, savePath, cookies, description) { root.doAddUrl(url, filename, savePath, cookies, description); }
    }

    // ── Add Batch dialog (FQT-023) \u2014 extracted to AddBatchDialog.qml ──
    AddBatchDialog {
        id: addBatchDialog
        rootWidth: root.width
        rootHeight: root.height
        onAddBatch: function(text) { root.doAddBatch(text); }
    }

    // ── Exit-while-active dialog (FQT-024) \u2014 extracted to ExitDialog.qml ──
    ExitDialog {
        id: exitDialog
        rootWidth: root.width
        rootHeight: root.height
        onMinimize: function() { root.visible = false; }
        onExitKeepRunning: function() { root.quitApp(); }
        onExitPauseAll: function() { root.requestBus("pause_all_downloads", {}, function() { root.quitApp(); }); }
    }

    // ── Add Torrent / Magnet dialog (FQT-031) \u2014 extracted to AddTorrentDialog.qml ──
    AddTorrentDialog {
        id: addTorrentDialog
        rootWidth: root.width
        rootHeight: root.height
        probeResult: root.addTorrentProbeJson ? (function() { try { return JSON.parse(root.addTorrentProbeJson); } catch(e) { return null; } })() : null
        onProbe: function(input) { root.doProbeTorrent(input); }
        onStage: function(saveRoot) { root.doStageTorrent(saveRoot); }
    }

    // ── Delete torrent confirm dialog (FQT-032) \u2014 extracted to DeleteTorrentDialog.qml ──
    DeleteTorrentDialog {
        id: deleteTorrentDialog
        rootWidth: root.width
        rootHeight: root.height
        deleteCount: root.pendingDeleteTorrentIds.length
        onConfirm: function() { root.doDeleteTorrentConfirm(); }
        onCancel: function() { root.pendingDeleteTorrentIds = []; deleteTorrentDialog.visible = false; }
    }

    // ── FQT-080: Preferences dialog (tabbed) ─────────────────────────────
    property var prefsData: ({})

    function loadPreferences() {
        requestBus("get_settings", {}, function(raw) {
            try {
                var parsed = JSON.parse(raw);
                if (parsed.error) {
                    // Cold start: the daemon is still coming up. Stay quiet and let the
                    // reload on `session.connected` fill the settings in.
                    if (session.connected) showError(parsed.error);
                    return;
                }
                root.prefsData = parsed;
        Theme.motion = (parsed["reduce_motion"] || "0") !== "1";
                if (root.autostartLaunch && !root.autostartResolved) {
                    root.autostartResolved = true;
                    if (parsed["start_minimized"] !== "1") { root.visible = true; root.raise(); root.requestActivate(); }
                }
                root.dropTargetShown = (parsed["show_drop_target"] || "0") === "1";
            } catch(e) {
                showError("Invalid settings response");
                return;
            }
            root.clipboardMonitorEnabled = (root.prefsData["clipboard_monitor_enabled"] || "0") === "1";
            root.lastKillSwitchState = (root.prefsData["torrent_kill_switch_active"] || "0");
            try {
                root.downloadVisibleColumns = JSON.parse(root.prefsData["download_visible_columns"] || JSON.stringify(root.defaultDownloadColumns));
            } catch(e) { root.downloadVisibleColumns = root.defaultDownloadColumns; }
            try {
                root.torrentVisibleColumns = JSON.parse(root.prefsData["torrent_visible_columns"] || "[]");
            } catch(e) { root.torrentVisibleColumns = []; }
            try {
                root.applyTorrentColumnWidths(JSON.parse(root.prefsData["torrent_column_widths"] || "[]"));
            } catch(e) {}
            loadAnonymityScore();
        });
    }

    function savePref(key, value) {
        requestBus("set_setting", { key: key, value: value }, function(err) {
            if (err) {
                showError(err);
                return;
            }
            if (key === "product_mode") {
                root.loadPreferences();
                return;
            }
            // A new object, so every binding that reads a preference notices the change.
            var updated = Object.assign({}, root.prefsData);
            updated[key] = value;
            root.prefsData = updated;
            if (key === "torrent_kill_switch_active")
                root.lastKillSwitchState = value || "0";
        });
    }

    // Reusable checkbox component inline
    Component {
        id: prefCheckBox
        Row {
            property string prefKey
            property string label
            property string tooltip
            spacing: Theme.sp2; height: Theme.rowH
            Rectangle {
                width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                property bool checked: (root.prefsData[prefKey] || "0") === "1"
                color: checked ? Theme.accent : Theme.bg
                border.color: Theme.surfaceAlt; border.width: 1
                Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: root.savePref(prefKey, parent.checked ? "0" : "1") }
            }
            Text { anchors.verticalCenter: parent.verticalCenter; text: label; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
        }
    }

    // FQT-080: Preferences dialog \u2014 extracted to PrefsDialog.qml (Phase 2 split #3)
    PrefsDialog {
        id: prefsDialog
        torrentsVisible: root.torrentsEnabled
        bus: bus
        requestBus: root.requestBus
        busError: root.busError
        session: session
        prefsData: root.prefsData
        clipboardMonitorEnabled: root.clipboardMonitorEnabled
        ferricHostRegistered: root.ferricHostRegistered
        anonymityScore: root.anonymityScore
        anonymityGrade: root.anonymityGrade
        savePref: root.savePref
        showError: root.showError
        showInfo: root.showInfo
        openWebhooks: function() { root.loadWebhooks(); webhooksDialog.visible = true; }
        toggleClipboardMonitor: function() {
            root.clipboardMonitorEnabled = !root.clipboardMonitorEnabled;
            root.savePref("clipboard_monitor_enabled", root.clipboardMonitorEnabled ? "1" : "0");
        }
    }


    // ── FQT-082: Proxy Manager \u2014 extracted to ProxyDialog / ProxyAddDialog ─
    property var proxyList: []

    function loadProxies() {
        requestBus("get_proxies", {}, function(raw) {
            try { root.proxyList = JSON.parse(raw); } catch(e) { root.proxyList = []; }
        });
    }

    ProxyDialog {
        id: proxyDialog
        bus: bus
        requestBus: root.requestBus
        proxyList: root.proxyList
        loadProxies: root.loadProxies
        showError: root.showError
        showInfo: root.showInfo
        openAdd: function() { proxyAddDialog.openEmpty(); }
    }

    ProxyAddDialog {
        id: proxyAddDialog
        bus: bus
        requestBus: root.requestBus
        busError: root.busError
        loadProxies: root.loadProxies
        showError: root.showError
    }

    // ── FQT-085: Webhooks dialog \u2014 extracted to WebhooksDialog.qml ─────
    property var webhookList: []

    function loadWebhooks() {
        requestBus("get_webhooks", {}, function(raw) {
            try { root.webhookList = JSON.parse(raw); } catch(e) { root.webhookList = []; }
        });
    }

    WebhooksDialog {
        id: webhooksDialog
        bus: bus
        requestBus: root.requestBus
        busError: root.busError
        webhookList: root.webhookList
        loadWebhooks: root.loadWebhooks
        showError: root.showError
        showInfo: root.showInfo
    }

    // FQT-088: About dialog \u2014 extracted to AboutDialog.qml
    AboutDialog {
        id: aboutDialog
        bus: bus
        requestBus: root.requestBus
        session: session
        showError: root.showError
    }

    // FQT-089: Keyboard shortcuts dialog \u2014 extracted to ShortcutsDialog.qml
    ShortcutsDialog {
        id: shortcutsDialog
    }

    // ── FQT-103: Stats window \u2014 extracted to StatsDialog.qml ──────────
    property var statsData: ({})

    function loadStats() {
        requestBus("get_stats", {}, function(raw) {
            try { root.statsData = JSON.parse(raw); } catch(e) { root.statsData = {}; }
            root.statsData.download_bps = hub.download_bps || 0;
        });
    }

    StatsDialog {
        id: statsDialog
        session: session
        statsData: root.statsData
        loadStats: root.loadStats
        formatBps: root.formatBps
        formatUptime: root.formatUptime
    }

    // ── FQT-073: Column chooser \u2014 extracted to ColumnChooserDialog.qml ─
    function toggleDownloadColumn(title) {
        var vis = root.downloadVisibleColumns.slice();
        var idx = vis.indexOf(title);
        if (idx >= 0) vis.splice(idx, 1); else vis.push(title);
        root.downloadVisibleColumns = vis;
        root.savePref("download_visible_columns", JSON.stringify(vis));
    }
    function toggleTorrentColumn(title) {
        var vis = root.torrentVisibleColumns.slice();
        var idx = vis.indexOf(title);
        if (idx >= 0) vis.splice(idx, 1); else vis.push(title);
        root.torrentVisibleColumns = vis;
        root.savePref("torrent_visible_columns", JSON.stringify(vis));
    }

    ColumnChooserDialog {
        id: columnChooserDialog
        downloadColumnsAll: root.downloadColumnsAll
        downloadVisibleColumns: root.downloadVisibleColumns
        torrentColumnsAll: root.torrentColumnsAll
        torrentVisibleColumns: root.torrentVisibleColumns
        toggleDownloadColumn: root.toggleDownloadColumn
        toggleTorrentColumn: root.toggleTorrentColumn
    }

    function formatUptime(secs) {
        var h = Math.floor(secs / 3600);
        var m = Math.floor((secs % 3600) / 60);
        var s = secs % 60;
        return h + "h " + m + "m " + s + "s";
    }

    // ── FQT-090: Overview workspace ─────────────────────────────────────
    property var overviewStats: ({})
    property var overviewRecent: []

    // Status-bar totals come from overviewStats, which was loaded once at startup and then
    // drifted (it said 1/6 DL with nothing running). Re-read them on every list refresh.
    function refreshStatusStats() {
        requestBus("get_stats", {}, function(raw) {
            try {
                var s = JSON.parse(raw);
                if (s.error) return;
                s.download_bps = hub.download_bps || 0;
                root.overviewStats = s;
            } catch(e) {}
        });
    }

    function loadOverview() {
        requestBus("get_stats", {}, function(raw) {
            try { root.overviewStats = JSON.parse(raw); } catch(e) { root.overviewStats = {}; }
            root.overviewStats.download_bps = hub.download_bps || 0;
        });
        requestBus("get_downloads", {}, function(raw) {
            try { root.overviewRecent = JSON.parse(raw).filter(function(d) { return !root.pendingCaptures[d.id]; }).slice(0, 10); }
            catch(e) { root.overviewRecent = []; }
        });
        requestBus("get_settings", {}, function(raw) {
            try {
                var s = JSON.parse(raw);
                var pd = {};
                for (var k in root.prefsData) pd[k] = root.prefsData[k];
                var keys = ["product_mode", "torrent_dht_enabled", "torrent_pex_enabled",
                            "torrent_lsd_enabled", "torrent_tcp_listen_enabled", "torrent_proxy_required"];
                for (var i = 0; i < keys.length; i++) {
                    if (s[keys[i]] !== undefined) pd[keys[i]] = s[keys[i]];
                }
                if (!pd["product_mode"]) pd["product_mode"] = "balanced";
                root.prefsData = pd;
            } catch (e2) {}
        });
        loadAnonymityScore();
        loadPrivacyDashboard();
    }

    // ── FQT-036: Torrent Policy dialog \u2014 extracted to TorrentPolicyDialog.qml ─
    TorrentPolicyDialog {
        id: torrentPolicyDialog
        policyData: root.torrentPolicyData
        togglePolicyField: root.togglePolicyField
        doPrivacyCheck: root.doPrivacyCheck
        saveTorrentPolicy: root.saveTorrentPolicy
    }

    // ── FQT-036: Privacy check result toast ──────────────────────────────
    Rectangle {
        id: privacyCheckResult
        property bool passed: false
        property var violations: []
        property var warnings: []
        // Warnings never block start, but a green "passed" while exposed is a lie.
        readonly property color statusColor: !passed ? Theme.danger
            : (warnings.length > 0 ? Theme.warning : Theme.success)
        visible: false; z: 110
        anchors { horizontalCenter: parent.horizontalCenter; bottom: parent.bottom; bottomMargin: Theme.sp5 }
        width: Math.min(680, Math.max(300, privacyCheckText.implicitWidth + 40))
        height: Math.max(40, privacyCheckText.implicitHeight + 20); radius: Theme.radius
        color: Theme.surface
        border.color: statusColor; border.width: 1

        Text {
            id: privacyCheckText
            anchors.centerIn: parent
            width: parent.width - 40
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            color: privacyCheckResult.statusColor
            font.pixelSize: Theme.fontSizeSmall
            // Headline then detail, separated by a plain space: the backend
            // strings carry their own em-dash, so joining with another one reads
            // as a formatting glitch.
            text: {
                if (!privacyCheckResult.passed)
                    return "Privacy violations: " + privacyCheckResult.violations.join(" ")
                if (privacyCheckResult.warnings.length === 0)
                    return "Privacy check passed"
                return "Privacy check passed, but not anonymous. "
                    + privacyCheckResult.warnings.join(" ")
            }
        }

        MouseArea { anchors.fill: parent; onClicked: privacyCheckResult.visible = false }

        // Restarted explicitly by doPrivacyCheck rather than bound to `visible`, so
        // re-running the check while the toast is still up gives the new result a
        // full dwell instead of inheriting the tail of the old one.
        Timer {
            id: privacyToastTimer
            interval: privacyCheckResult.warnings.length > 0 || !privacyCheckResult.passed ? 12000 : 5000
            onTriggered: privacyCheckResult.visible = false
        }
    }

    // ── FQT-044: Add Category dialog \u2014 extracted to AddCategoryDialog.qml ──
    AddCategoryDialog {
        id: addCategoryDialog
        rootWidth: root.width
        rootHeight: root.height
        onAdd: function(name) { root.doAddCategory(name); }
    }

    // ── FQT-045: Edit Queue dialog \u2014 extracted to EditQueueDialog.qml ─
    EditQueueDialog {
        id: editQueueDialog
        doSaveQueue: root.doSaveQueue
        queueList: root.queues
        currentQueueId: root.editingQueue ? root.editingQueue.id : ""
        onPickQueue: function(q) { root.doEditQueue(q) }
    }

    // ── FQT-046: Site Rules dialog \u2014 extracted to SiteRuleDialog.qml ─
    SiteRuleDialog {
        id: siteRuleDialog
        bus: bus
        requestBus: root.requestBus
        siteRules: root.siteRules
        ruleDialogMode: root.ruleDialogMode
        editingRule: root.editingRule
        loadSiteRules: root.loadSiteRules
        doAddSiteRule: root.doAddSiteRule
        doEditSiteRule: root.doEditSiteRule
        doDeleteSiteRule: root.doDeleteSiteRule
        showError: root.showError
        resetMode: function() { root.ruleDialogMode = "add"; root.editingRule = null; }
    }

    // ── FQT-068: Add RSS dialog \u2014 extracted to AddRssDialog.qml ──
    AddRssDialog {
        id: addRssDialog
        rootWidth: root.width
        rootHeight: root.height
        onAdd: function(url) { root.doAddRssFeed(url); }
    }

    // ── FQT-102: Add RSS Rule dialog \u2014 extracted to AddRssRuleDialog.qml ──
    ListModel { id: rssRuleFeedModel }

    AddRssRuleDialog {
        id: addRssRuleDialog
        rootWidth: root.width
        rootHeight: root.height
        feedModel: rssRuleFeedModel
        onAdd: function(feedId, name, pattern) {
            requestBus("add_rss_rule", {
                feed_id: feedId,
                name: name,
                pattern: pattern,
                action: "download"
            }, function(raw) {
                try {
                    var result = JSON.parse(raw)
                    if (result.error) { showError(result.error); return }
                    addRssRuleDialog.visible = false
                    root.loadRssRules()
                } catch(e) { showError("Add RSS rule failed: " + e) }
            })
        }
        showError: root.showError
    }

    // ── FQT-065: Trackers edit dialog \u2014 extracted to TrackersEditDialog.qml ──
    TrackersEditDialog {
        id: trackersEditDialog
        rootWidth: root.width
        rootHeight: root.height
        onSave: function(text) {
            if (selectedTorrents.length !== 1) return;
            requestBus("set_torrent_trackers", {
                id: selectedTorrents[0].id,
                trackers: text
            }, function(raw) {
                try {
                    var res = JSON.parse(raw);
                    if (res.error) { showError(res.error); return; }
                    trackersEditDialog.visible = false;
                    refreshTorrentDetail();
                } catch(e) { showError("Tracker update failed: " + e); }
            });
        }
    }

    // ── FQT-063: Set Location dialog \u2014 extracted to SetLocationDialog.qml ──
    SetLocationDialog {
        id: setLocationDialog
        rootWidth: root.width
        rootHeight: root.height
        onApply: function(path) { root.doApplySetLocation(); }
    }

    // ── FQT-048: Duplicate URL dialog \u2014 extracted to DuplicateUrlDialog.qml ──
    DuplicateUrlDialog {
        id: duplicateUrlDialog
        rootWidth: root.width
        rootHeight: root.height
        duplicateUrl: root.duplicateUrlLabel || ""
        onAddAnyway: function() { root.doAddDuplicateConfirm(); }
    }

    // IDM's drop target: drag a link from the browser (or a text selection) onto the window to add it.
    function handleDroppedLink(drop) {
        var text = drop.hasUrls && drop.urls.length > 0 ? drop.urls[0].toString() : (drop.hasText ? drop.text : "");
        text = text.trim().split(/[\r\n]+/)[0].trim();
        if (!/^(https?|ftp):\/\//i.test(text)) return;
        addUrlDialog.clearInputs();
        addUrlDialog.urlText = text;
        addUrlDialog.visible = true;
        drop.accept();
    }
    DropArea {
        anchors.fill: parent
        z: 1
        onDropped: function(drop) { root.handleDroppedLink(drop) }
    }

    // IDM's floating "drop target" (Tasks > Show drop target): a small always-on-top window to drop links on.
    property bool dropTargetShown: false
    function setDropTarget(on) {
        root.dropTargetShown = on;
        root.savePref("show_drop_target", on ? "1" : "0");
    }
    Window {
        id: dropTargetWindow
        visible: root.dropTargetShown
        flags: Qt.Tool | Qt.WindowStaysOnTopHint | Qt.WindowCloseButtonHint
        title: qsTr("Ferric drop target")
        width: 96; height: 96
        minimumWidth: width; maximumWidth: width; minimumHeight: height; maximumHeight: height
        x: Screen.width - width - 48
        y: Screen.height - height - 140
        color: Theme.bg
        onClosing: function(close) { root.setDropTarget(false) }
        Image { anchors.centerIn: parent; width: 48; height: 48; source: "qrc:/icons/ferric-tray.png"; fillMode: Image.PreserveAspectFit }
        Text { anchors { horizontalCenter: parent.horizontalCenter; bottom: parent.bottom; bottomMargin: Theme.sp1 }
            text: qsTr("Drop links here"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
        DropArea {
            anchors.fill: parent
            onDropped: function(drop) { root.handleDroppedLink(drop) }
        }
    }

    // ── FQT-047: Clipboard offer toast \u2014 extracted to ClipboardOfferToast.qml ──
    ClipboardOfferToast {
        id: clipboardOfferToast
        offerType: root.clipboardOfferType
        offerUrl: root.clipboardOfferUrl
        onAdd: function() { root.doClipboardOfferAdd(); }
    }
}
