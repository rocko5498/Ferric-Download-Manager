import QtQuick
import QtQuick.Controls as QQC
import Qt.labs.platform as Platform
import com.ferric.app 1.0

// FQT-080: Preferences dialog \u2014 extracted from main.qml (Phase 2 split #3)
Rectangle {
    id: prefsDialog

    visible: false
    z: 100
    anchors.fill: parent
    color: "#CC000000"
    focus: true

    onVisibleChanged: if (visible) {
        if (Theme.motion) prefsOpen.restart()
        loadSiteLogins()
        Qt.callLater(function() {
            if (prefsDialog.visible)
                prefsDialog.forceActiveFocus()
        })
    }

    ParallelAnimation {
        id: prefsOpen
        NumberAnimation { target: prefsCard; property: "opacity"; from: 0; to: 1; duration: Theme.animMs; easing.type: Easing.OutCubic }
    }
    property var siteLogins: []
    function loadSiteLogins() {
        if (!requestBus) return
        requestBus("get_site_logins", {}, function(raw) {
            try { var v = JSON.parse(raw); prefsDialog.siteLogins = Array.isArray(v) ? v : [] } catch (e) { prefsDialog.siteLogins = [] }
        })
    }
    function addSiteLogin(server, user, pass) {
        if (server === "" || user === "") return
        requestBus("save_site_login", { body_json: JSON.stringify({ server: server, username: user, password: pass }) }, function(raw) {
            var err = busError(raw)
            if (err) showError(err)
            prefsDialog.loadSiteLogins()
        })
    }
    function removeSiteLogin(id) {
        requestBus("delete_site_login", { id: id }, function() { prefsDialog.loadSiteLogins() })
    }
    Keys.onEscapePressed: prefsDialog.visible = false

    property var bus
    property var requestBus
    property var busError
    property bool appModal: true
    property var session
    property var prefsData: ({})
    property int prefsTab: 0
    property bool torrentsVisible: false
    property bool clipboardMonitorEnabled: false
    property bool ferricHostRegistered: false
    property var anonymityScore
    property string anonymityGrade: ""

    property var savePref
    property var openWebhooks
    property var showError
    property var showInfo
    property var toggleClipboardMonitor

    // formatAnonymityScore removed \u2014 use inline: (anonymityScore ?? "...") + "/100"
        Rectangle { x: prefsCard.x - Theme.sp1; y: prefsCard.y + Theme.sp2; width: prefsCard.width + 2 * Theme.sp1; height: prefsCard.height + Theme.sp1; radius: prefsCard.radius + Theme.sp1; color: "#14000000"; opacity: prefsCard.opacity }
        Rectangle { x: prefsCard.x; y: prefsCard.y + Theme.sp1; width: prefsCard.width; height: prefsCard.height; radius: prefsCard.radius; color: Theme.shadow; opacity: prefsCard.opacity }
        Rectangle {
            id: prefsCard
            width: Math.min(parent.width - 2 * Theme.sp5, Theme.dialogW)
            height: Math.min(parent.height - 2 * Theme.sp5, 987)
            anchors.centerIn: parent
            color: Theme.surface; radius: Theme.radiusCard

            Column {
                anchors { fill: parent; margins: Theme.sp4 }
                spacing: Theme.sp3

                Text { id: prefsTitle; height: Theme.sp5 - Theme.sp2; text: qsTr("Preferences"); color: Theme.text; font.pixelSize: Theme.fontSizeMedium; font.bold: true }

                // Tab bar
                Row {
                    id: prefsTabs
                    width: parent.width; height: Theme.controlH; spacing: Theme.sp1
                    Repeater {
                        model: ["General", "Downloads", "Connection", "Speed", "BitTorrent", "Advanced"]
                        Rectangle {
                            visible: prefsDialog.torrentsVisible || (modelData !== "Connection" && modelData !== "BitTorrent")
                            width: Math.max(Theme.btnM, tabLabel.implicitWidth + Theme.sp4); height: Theme.controlH; radius: Theme.radius
                            color: prefsTab === index ? Theme.accent : (tabArea.containsMouse ? Theme.surfaceAlt : Theme.bg)
                            Text { id: tabLabel; anchors.centerIn: parent; text: modelData; color: prefsTab === index ? "#fff" : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            MouseArea { id: tabArea; anchors.fill: parent; hoverEnabled: true; onClicked: prefsTab = index }
                        }
                    }
                }

                // Tab content
                Flickable {
                    width: parent.width; height: parent.height - prefsTitle.height - prefsTabs.height - prefsFooter.height - 3 * parent.spacing
                    clip: true; contentHeight: tabContent.height
                    boundsBehavior: Flickable.StopAtBounds
                    QQC.ScrollBar.vertical: QQC.ScrollBar {}
                    Column { id: tabContent; width: parent.width; spacing: Theme.sp3

                        // ── General tab ─────────────────────────────────
                        Item { visible: prefsTab === 0; width: parent.width; height: visible ? genCol.height : 0; clip: true
                        Column { id: genCol; width: parent.width; spacing: Theme.sp2
                            Text { text: qsTr("Application"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            // Language
                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Language:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 120 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 140; height: Theme.controlH; color: Theme.bg; border.color: langInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: langInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                                        text: prefsData["ui_language"] || "en-US"
                                        onEditingFinished: savePref("ui_language", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("en-US"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("Network"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["allow_lan_downloads"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("allow_lan_downloads", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Allow downloads from my local network (NAS, router: 10.x, 172.16-31.x, 192.168.x.x)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("Startup"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["auto_start"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("auto_start", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Start with operating system"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["start_minimized"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("start_minimized", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Start minimized to system tray"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["close_to_tray"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("close_to_tray", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Close button minimizes to tray instead of exit"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["confirm_on_exit"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("confirm_on_exit", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Confirm on exit when downloads are active"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["single_instance"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("single_instance", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Allow only one instance of Ferric"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius; anchors.verticalCenter: parent.verticalCenter
                                    property bool checked: (prefsData["reduce_motion"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("reduce_motion", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Reduce animations"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            // ── FQT-086: Portable mode ─────────────────
                            Text { text: qsTr("Portable mode"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["portable_mode"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("portable_mode", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Portable mode (data dir next to binary)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("When portable mode is on, data is stored next to the executable. Requires restart to take effect."); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width }

                            // ── Clipboard ────────────────────────────────
                            Text { text: qsTr("Clipboard"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: clipboardMonitorEnabled
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: { if (toggleClipboardMonitor) toggleClipboardMonitor(); } }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Monitor clipboard for URLs and magnet links"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            // ── Notifications ───────────────────────────
                            Text { text: qsTr("Notifications"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["notifications_enabled"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("notifications_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Show notifications on download complete/error"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["sound_on_complete"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("sound_on_complete", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Play a sound when all downloads finish"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["sound_on_each_done"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("sound_on_each_done", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Play a sound when each file finishes"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["sound_on_error"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("sound_on_error", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Play a sound when a download fails"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            // ── Browser Integration ─────────────────────
                            Text { text: qsTr("Browser Integration"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4
                                    color: ferricHostRegistered ? Theme.success : Theme.danger
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: ferricHostRegistered ? "Connected to your browsers" : "Not connected to your browsers"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: ferricHostRegistered ? "The Ferric extension can hand downloads to this app." : "Reinstall Ferric to connect your browsers, then reload the Ferric extension in each one."; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width }
                        }
                        } // General tab Item

                        // ── Downloads tab ───────────────────────────────
                        Item { visible: prefsTab === 1; width: parent.width; height: visible ? dlCol.height : 0; clip: true
                        Column { id: dlCol; width: parent.width; spacing: Theme.sp2
                            Text { text: qsTr("Download defaults"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            // Default save path
                            Column { width: parent.width; spacing: Theme.sp1
                                Text { text: qsTr("Default save path"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                                Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: defaultSaveDirInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: defaultSaveDirInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                                        text: prefsData["default_save_dir"] || ""
                                        onEditingFinished: savePref("default_save_dir", text) } }
                            }

                            // IDM's scheduler: "Number of retries for each file if downloading failed".
                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Retries per file if downloading fails"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: retriesInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: retriesInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                                        inputMethodHints: Qt.ImhDigitsOnly
                                        text: prefsData["max_retries"] || "4"
                                        onEditingFinished: savePref("max_retries", text) } }
                            }

                            // IDM's scheduler "when done": what to do once every download has finished.
                            Text { text: qsTr("When all downloads finish"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
                            Row {
                                spacing: Theme.sp2
                                height: Theme.controlH
                                Repeater {
                                    model: [
                                        { id: "none", label: qsTr("Do nothing") },
                                        { id: "exit", label: qsTr("Exit Ferric") },
                                        { id: "shutdown", label: qsTr("Turn off the computer") }
                                    ]
                                    Rectangle {
                                        width: doneLabel.implicitWidth + 16
                                        height: Theme.controlH
                                        radius: Theme.radius
                                        property bool selected: (prefsData["when_done_action"] || "none") === modelData.id
                                        color: selected ? Theme.accent : Theme.surface
                                        Text { id: doneLabel; anchors.centerIn: parent; text: modelData.label; color: parent.selected ? "white" : Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                        MouseArea { anchors.fill: parent; onClicked: savePref("when_done_action", modelData.id) }
                                    }
                                }
                            }
                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { text: qsTr("Open this file when done:"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 240; height: Theme.controlH; color: Theme.bg; border.color: Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { clip: true; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                                        text: prefsData["open_file_when_done"] || ""
                                        onEditingFinished: savePref("open_file_when_done", text.trim()) } }
                            }
                            Text { width: parent.width; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                                text: qsTr("Turning the computer off waits 60 seconds and shows a Cancel button first.") }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["adaptive_connections"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("adaptive_connections", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Open more connections on big files while the speed keeps growing, up to"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                Rectangle { width: 40; height: Theme.rowH; color: Theme.bg; radius: Theme.radius; border.color: Theme.surfaceAlt
                                    TextInput { clip: true; anchors.fill: parent; anchors.margins: Theme.sp1; color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                                        inputMethodHints: Qt.ImhDigitsOnly
                                        text: prefsData["adaptive_max_connections"] || "24"
                                        onEditingFinished: savePref("adaptive_max_connections", text.trim()) } }
                            }
                            Row { spacing: Theme.sp1; height: Theme.rowH
                                Text { text: qsTr("Download limits: stop all downloads after"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter }
                                Rectangle { width: 56; height: Theme.rowH; color: Theme.bg; radius: Theme.radius; border.color: Theme.surfaceAlt
                                    TextInput { clip: true; anchors.fill: parent; anchors.margins: Theme.sp1; color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                                        inputMethodHints: Qt.ImhDigitsOnly
                                        text: prefsData["quota_mb"] || ""
                                        onEditingFinished: savePref("quota_mb", text.trim()) } }
                                Text { text: qsTr("MB within"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter }
                                Rectangle { width: 32; height: Theme.rowH; color: Theme.bg; radius: Theme.radius; border.color: Theme.surfaceAlt
                                    TextInput { clip: true; anchors.fill: parent; anchors.margins: Theme.sp1; color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                                        inputMethodHints: Qt.ImhDigitsOnly
                                        text: prefsData["quota_hours"] || ""
                                        onEditingFinished: savePref("quota_hours", text.trim()) } }
                                Text { text: qsTr("hours (1-23; empty = off)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter }
                            }

                            Text { text: qsTr("Proxy for downloads"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
                            Column { spacing: Theme.sp1; width: parent.width
                                Text { text: qsTr("Proxy server (http://host:port or socks5://user:pass@host:port; empty = none)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                                Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: dlProxyInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: dlProxyInput; clip: true; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                                        text: prefsData["proxy_url"] || ""
                                        onEditingFinished: savePref("proxy_url", text.trim()) } }
                                Text { text: qsTr("Automatic configuration script (PAC) address, used when no proxy server is set"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                                Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: dlPacInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: dlPacInput; clip: true; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                                        text: prefsData["pac_url"] || ""
                                        onEditingFinished: savePref("pac_url", text.trim()) } }
                                Row { spacing: Theme.sp2; height: Theme.rowH
                                    Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                        property bool checked: (prefsData["pac_from_windows"] || "0") === "1"
                                        color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                        Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                        MouseArea { anchors.fill: parent; onClicked: savePref("pac_from_windows", parent.checked ? "0" : "1") }
                                    }
                                    Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Use the script address from Windows' Internet settings when none is given above"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                }
                            }
                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["show_queue_panel"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("show_queue_panel", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Ask which queue when pressing Download Later"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }
                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["only_add_to_queue"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("only_add_to_queue", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Do not start downloading, only add files to the queue"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }
                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["ignore_mtime_changes"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("ignore_mtime_changes", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Ignore file modification time changes when resuming a download"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }
                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["file_time_from_server"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("file_time_from_server", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Set the file date to the date the server reports"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            // IDM's Options -> Sites Logins: a user name and password kept per server.
                            Text { text: qsTr("Site logins"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
                            Repeater {
                                model: prefsDialog.siteLogins
                                Row {
                                    spacing: Theme.sp2; height: Theme.controlH
                                    Text { text: modelData.server + "  \u2022  " + modelData.username; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter
                                        MouseArea { anchors.fill: parent; onClicked: { slServer.text = modelData.server; slUser.text = modelData.username; slPass.text = ""; slPass.forceActiveFocus() } } }
                                    Rectangle { width: 54; height: Theme.rowH; radius: Theme.radius; color: Theme.surfaceAlt
                                        Text { anchors.centerIn: parent; text: qsTr("Remove"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                        MouseArea { anchors.fill: parent; onClicked: prefsDialog.removeSiteLogin(modelData.id) } }
                                }
                            }
                            Row {
                                spacing: Theme.sp1; height: Theme.controlH
                                Rectangle { width: 150; height: Theme.controlH; color: Theme.bg; radius: Theme.radius; border.color: Theme.surfaceAlt
                                    TextInput { id: slServer; clip: true; anchors.fill: parent; anchors.margins: Theme.sp1
                                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                    Text { visible: slServer.text === ""
                                        anchors.fill: parent; anchors.margins: Theme.sp1; text: qsTr("server or server/path"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall } }
                                Rectangle { width: 90; height: Theme.controlH; color: Theme.bg; radius: Theme.radius; border.color: Theme.surfaceAlt
                                    TextInput { id: slUser; clip: true; anchors.fill: parent; anchors.margins: Theme.sp1
                                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                    Text { visible: slUser.text === ""
                                        anchors.fill: parent; anchors.margins: Theme.sp1; text: qsTr("user name"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall } }
                                Rectangle { width: 90; height: Theme.controlH; color: Theme.bg; radius: Theme.radius; border.color: Theme.surfaceAlt
                                    TextInput { id: slPass; clip: true; echoMode: TextInput.Password; anchors.fill: parent; anchors.margins: Theme.sp1
                                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                    Text { visible: slPass.text === ""
                                        anchors.fill: parent; anchors.margins: Theme.sp1; text: qsTr("password"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall } }
                                Rectangle { width: 44; height: Theme.controlH; radius: Theme.radius; color: Theme.accentDark
                                    Text { anchors.centerIn: parent; text: qsTr("Add"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: {
                                        prefsDialog.addSiteLogin(slServer.text.trim(), slUser.text.trim(), slPass.text)
                                        slServer.text = ""; slUser.text = ""; slPass.text = "" } } }
                            }
                            Text { width: parent.width; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                                text: qsTr("Used for downloads from that server when the link has no login of its own. Click an entry to change its login. Passwords are stored encrypted for your Windows account.") }

                            // IDM's Downloads options: which dialogs appear around a download
                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["show_start_dialog"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("show_start_dialog", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Show start download dialog (Download File Info)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }
                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["show_progress_dialog"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("show_progress_dialog", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Show download progress dialog"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }
                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["show_complete_dialog"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("show_complete_dialog", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Show download complete dialog"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["ask_where_to_save"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("ask_where_to_save", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Ask where to save each download"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["auto_resume_on_start"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("auto_resume_on_start", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Auto-resume incomplete downloads on startup"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["dedupe_enabled"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("dedupe_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Duplicate download detection"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["start_while_info_dialog"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("start_while_info_dialog", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Start downloading while the Download File Info dialog is open"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["duplicate_action"] || "number") === "overwrite"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("duplicate_action", parent.checked ? "number" : "overwrite") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("If the file already exists: replace it (unchecked: add a numbered name like file_1.zip)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["preallocate_disk"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("preallocate_disk", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Preallocate disk space"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("Connection limits"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Max connections/host:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 160 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: connPerHost.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: connPerHost; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["max_connections_per_host"] || "8"
                                        onEditingFinished: savePref("max_connections_per_host", text) } }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Max connections/download:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 160 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: connPerDl.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: connPerDl; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["max_connections_per_download"] || "8"
                                        onEditingFinished: savePref("max_connections_per_download", text) } }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Max concurrent:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 160 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: maxConc.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: maxConc; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["max_concurrent"] || "3"
                                        onEditingFinished: savePref("max_concurrent", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("0 = unlimited"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("HTTP client"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("User agent:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 120 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 380; height: Theme.controlH; color: Theme.bg; border.color: uaInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: uaInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                                        text: prefsData["user_agent"] || ""
                                        onEditingFinished: savePref("user_agent", text) } }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Connect timeout:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 120 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: timeoutInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: timeoutInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["connect_timeout_secs"] || "30"
                                        onEditingFinished: savePref("connect_timeout_secs", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("seconds"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }
                        }
                        } // Downloads tab Item

                        // ── Connection tab ──────────────────────────────
                        Item { visible: prefsTab === 2; width: parent.width; height: visible ? connCol.height : 0; clip: true
                        Column { id: connCol; width: parent.width; spacing: Theme.sp2
                            Text { text: qsTr("Torrent connection"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["torrent_tcp_listen_enabled"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("torrent_tcp_listen_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Allow incoming peer connections (TCP listen)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("When off, Ferric is outbound-only \u2014 no one can connect to you. Safer on public networks; may reduce swarm connectivity."); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Listen port:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 100 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: listenPortInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: listenPortInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["listen_port"] || "6881"
                                        onEditingFinished: savePref("listen_port", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("only when TCP listen enabled"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["upnp_enabled"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("upnp_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Enable UPnP/NAT-PMP port forwarding"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("Protocol"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { text: qsTr("Encryption:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 100 }
                                Repeater {
                                    model: ["disabled", "enabled", "forced"]
                                    Rectangle {
                                        width: 70; height: Theme.controlH; radius: Theme.radius
                                        color: (prefsData["protocol_encryption"] || "enabled") === modelData ? Theme.accent : Theme.bg
                                        border.color: Theme.surfaceAlt; border.width: 1
                                        Text { anchors.centerIn: parent; text: modelData; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                        MouseArea { anchors.fill: parent; onClicked: savePref("protocol_encryption", modelData) }
                                    }
                                }
                            }

                            Text { text: qsTr("Limits"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Global connections:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 100 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: globalConn.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: globalConn; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["max_connections_global"] || "200"
                                        onEditingFinished: savePref("max_connections_global", text) } }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Max uploads/torrent:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 100 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: maxUp.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: maxUp; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["max_uploads_per_torrent"] || "4"
                                        onEditingFinished: savePref("max_uploads_per_torrent", text) } }
                            }
                        }
                        } // Connection tab Item

                        // ── Speed tab ───────────────────────────────────
                        Item { visible: prefsTab === 3; width: parent.width; height: visible ? speedCol.height : 0; clip: true
                        Column { id: speedCol; width: parent.width; spacing: Theme.sp2
                            Text { text: qsTr("Global speed limits"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Download limit:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 110 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 80; height: Theme.controlH; color: Theme.bg; border.color: dlLimitInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: dlLimitInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["bandwidth_limit_bps"] || "0"
                                        onEditingFinished: savePref("bandwidth_limit_bps", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("bytes/sec (0 = unlimited)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Upload limit:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 110 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 80; height: Theme.controlH; color: Theme.bg; border.color: ulLimitInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: ulLimitInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["global_upload_limit_bps"] || "0"
                                        onEditingFinished: savePref("global_upload_limit_bps", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("bytes/sec (0 = unlimited)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("Alternative speed limits"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["alt_speed_enabled"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("alt_speed_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Enable alternative speed limits"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Alt DL limit:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 110 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 80; height: Theme.controlH; color: Theme.bg; border.color: altDlInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: altDlInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["alt_speed_download_bps"] || "0"
                                        onEditingFinished: savePref("alt_speed_download_bps", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("bytes/sec (0 = unlimited)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Alt UL limit:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 110 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 80; height: Theme.controlH; color: Theme.bg; border.color: altUlInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: altUlInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["alt_speed_upload_bps"] || "0"
                                        onEditingFinished: savePref("alt_speed_upload_bps", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("bytes/sec (0 = unlimited)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("Schedule"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Days:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 110 }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: prefsData["alt_speed_schedule_days"] || "every day"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("From:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 110 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: schedFrom.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: schedFrom; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                                        text: prefsData["alt_speed_schedule_from"] || "02:00"
                                        onEditingFinished: savePref("alt_speed_schedule_from", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("To:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: schedTo.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: schedTo; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                                        text: prefsData["alt_speed_schedule_to"] || "07:00"
                                        onEditingFinished: savePref("alt_speed_schedule_to", text) } }
                            }
                        }
                        } // Speed tab Item

                        // ── BitTorrent tab ──────────────────────────────
                        Item { visible: prefsTab === 4; width: parent.width; height: visible ? btCol.height : 0; clip: true
                        Column { id: btCol; width: parent.width; spacing: Theme.sp2
                            // T1 product modes \u2014 finished once: applies constellation via product_mode setting.
                            Text { text: qsTr("Network mode"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
                            Text {
                                width: parent.width
                                wrapMode: Text.WordWrap
                                color: Theme.textDim
                                font.pixelSize: Theme.fontSizeSmall
                                text: qsTr("Picking a mode updates DHT/PEX/LSD, inbound listen, proxy-required, and default seeding \u2014 and rewrites policy on existing public torrents.")
                            }
                            Row {
                                spacing: Theme.sp2
                                height: Theme.controlH
                                Repeater {
                                    model: [
                                        { id: "strict", label: qsTr("Strict") },
                                        { id: "balanced", label: qsTr("Balanced") },
                                        { id: "open", label: qsTr("Open") }
                                    ]
                                    Rectangle {
                                        width: modeLabel.implicitWidth + 16
                                        height: Theme.controlH
                                        radius: Theme.radius
                                        property bool selected: (prefsData["product_mode"] || "balanced") === modelData.id
                                        color: selected ? Theme.accent : Theme.surface
                                        border.color: selected ? Theme.warning : Theme.surfaceAlt
                                        border.width: 1
                                        Text {
                                            id: modeLabel
                                            anchors.centerIn: parent
                                            text: modelData.label
                                            color: selected ? "#fff" : Theme.text
                                            font.pixelSize: Theme.fontSizeSmall
                                        }
                                        MouseArea {
                                            anchors.fill: parent
                                            onClicked: savePref("product_mode", modelData.id)
                                        }
                                    }
                                }
                            }
                            Text {
                                width: parent.width
                                wrapMode: Text.WordWrap
                                color: Theme.textDim
                                font.pixelSize: Theme.fontSizeSmall
                                text: {
                                    var m = prefsData["product_mode"] || "balanced"
                                    if (m === "strict")
                                        return qsTr("Strict: no DHT/PEX/LSD, no inbound listen, proxy required, no default seeding.")
                                    if (m === "open")
                                        return qsTr("Open: full discovery + inbound listen + default seeding. Faster; more exposure.")
                                    return qsTr("Balanced: DHT+PEX on, outbound-only, proxy optional. Recommended default.")
                                }
                            }

                            Text { text: qsTr("Discovery"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["torrent_dht_enabled"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("torrent_dht_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Enable DHT (distributed hash table)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["torrent_pex_enabled"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("torrent_pex_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Enable PEX (peer exchange)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["torrent_lsd_enabled"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("torrent_lsd_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Enable LSD (local service discovery)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("Seeding"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["torrent_seeding_default"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("torrent_seeding_default", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Seed by default after download completes"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Ratio limit:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 80 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: seedRatioInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: seedRatioInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["torrent_seed_ratio_limit"] || "0"
                                        onEditingFinished: savePref("torrent_seed_ratio_limit", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("0 = unlimited"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Time limit (min):"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 80 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: seedTimeInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: seedTimeInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: { var secs = parseInt(prefsData["torrent_seed_time_limit_secs"] || "0"); return Math.round(secs / 60).toString(); }
                                        onEditingFinished: savePref("torrent_seed_time_limit_secs", (parseInt(text) * 60).toString()) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("0 = unlimited"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("Proxy (privacy)"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Column { width: parent.width; spacing: Theme.sp1
                                Text { text: qsTr("SOCKS5 proxy URL (e.g. socks5://127.0.0.1:1080)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                                Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: proxyUrlInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: proxyUrlInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                                        text: prefsData["torrent_proxy_url"] || ""
                                        onTextChanged: savePref("torrent_proxy_url", text) } }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["torrent_proxy_required"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: { var next = parent.checked ? "0" : "1"; savePref("torrent_proxy_required", next); if (next === "0") savePref("torrent_kill_switch_active", "0"); } }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Require proxy for all torrents (kill switch)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4
                                    color: (prefsData["torrent_kill_switch_active"] || "0") === "1" ? Theme.danger : Theme.success }
                                Text { anchors.verticalCenter: parent.verticalCenter; color: (prefsData["torrent_kill_switch_active"] || "0") === "1" ? Theme.danger : Theme.success; font.pixelSize: Theme.fontSizeSmall
                                    text: (prefsData["torrent_kill_switch_active"] || "0") === "1" ? "Kill switch ACTIVE \u2014 torrents paused, no proxy" : "Kill switch inactive \u2014 proxy available" }
                            }

                            Text { text: qsTr("When \"Require proxy\" is on and no SOCKS5 URL is set, all torrents are paused until a proxy is configured. For maximum privacy, use a VPN \u2014 Ferric's proxy is a best-effort boundary, not a leak-proof tunnel."); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width }

                            Text { text: qsTr("Import"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Column { width: parent.width; spacing: Theme.sp1
                                Text { text: qsTr("qBittorrent BT_backup folder (leave blank for default location)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                                Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: qbtImportDirInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: qbtImportDirInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal } }
                            clip: true
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Rectangle { width: 180; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
                                    Text { anchors.centerIn: parent; text: qsTr("Import from qBittorrent"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea {
                                        anchors.fill: parent
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: {
                                            qbtImportResultText.text = qsTr("Importing\u2026");
                                            requestBus("import_qbittorrent", { directory: qbtImportDirInput.text }, function(raw) {
                                                try {
                                                    var res = JSON.parse(raw);
                                                    if (res.error) { qbtImportResultText.text = "Error: " + res.error; return; }
                                                    qbtImportResultText.text = res.imported + " / " + res.found + " torrents imported";
                                                    downloadModel.refresh();
                                                    torrentModel.refresh();
                                                } catch (e) {
                                                    qbtImportResultText.text = qsTr("Import failed \u2014 could not reach daemon");
                                                }
                                            });
                                        }
                                    }
                                }
                                Text { id: qbtImportResultText; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; verticalAlignment: Text.AlignVCenter; height: Theme.controlH }
                            }
                        }
                        } // BitTorrent tab Item

                        // ── Advanced tab ────────────────────────────────
                        Item { visible: prefsTab === 5; width: parent.width; height: visible ? advCol.height : 0; clip: true
                        Column { id: advCol; width: parent.width; spacing: Theme.sp2
                            Text { text: qsTr("Capture"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Min capture size:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 120 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 80; height: Theme.controlH; color: Theme.bg; border.color: minCapInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: minCapInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["min_capture_size"] || "0"
                                        onEditingFinished: savePref("min_capture_size", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("bytes (0 = capture all)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("Chunks"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Chunk size:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 120 }
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 60; height: Theme.controlH; color: Theme.bg; border.color: chunkSizeInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                                    TextInput { id: chunkSizeInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly
                                        text: prefsData["chunk_size_kb"] || "512"
                                        onEditingFinished: savePref("chunk_size_kb", text) } }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("KB"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Text { text: qsTr("Audit / Webhooks"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["webhook_enabled"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("webhook_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Enable webhooks"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Rectangle {
                                    width: 120; height: Theme.controlH; radius: Theme.radius
                                    color: manageWhArea.containsMouse ? Theme.border : Theme.surfaceAlt
                                    Text { anchors.centerIn: parent; text: qsTr("Manage Webhooks"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { id: manageWhArea; anchors.fill: parent; hoverEnabled: true
                                        onClicked: { if (openWebhooks) openWebhooks(); }
                                    }
                                }
                            }

                            Text { text: qsTr("Security"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["av_enabled"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("av_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Run AV scan after download completes"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            Row { visible: prefsDialog.torrentsVisible; spacing: Theme.sp2; height: prefsDialog.torrentsVisible ? 22 : 0
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["torrent_engine_enabled"] || "1") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("torrent_engine_enabled", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Torrent engine master switch (disable to block all torrent starts)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            // ── FQT-081: Privacy & Audit ───────────────
                            Text { text: qsTr("Privacy & Audit"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4; color: Theme.success }
                                Text { anchors.verticalCenter: parent.verticalCenter; id: anonScoreText; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; text: qsTr("Anonymity score: ") + ((anonymityScore !== null && anonymityScore !== undefined) ? anonymityScore + "/100" : "...") + " - " + anonymityGrade }
                            }

                            Row { spacing: Theme.sp2; height: Theme.controlH
                                Rectangle {
                                    width: 110; height: Theme.controlH; radius: Theme.radius
                                    color: exportAuditArea.containsMouse ? Theme.border : Theme.surfaceAlt
                                    Text { anchors.centerIn: parent; text: qsTr("Export audit log"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { id: exportAuditArea; anchors.fill: parent; hoverEnabled: true
                                        onClicked: {
                                            requestBus("export_audit", {}, function(raw) {
                                                try {
                                                    pendingAuditJson = JSON.stringify(JSON.parse(raw), null, 2);
                                                    auditExportFileDialog.open();
                                                } catch(e) { showError("Export failed: " + e); }
                                            });
                                        }
                                    }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("JSON \u2014 cookies/credentials never included"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            }

                            property string pendingAuditJson: ""

                            Platform.FileDialog {
                                id: auditExportFileDialog
                                title: qsTr("Save Audit Log")
                                fileMode: Platform.FileDialog.SaveFile
                                nameFilters: ["JSON files (*.json)"]
                                onAccepted: {
                                    var err = bus.write_text_file(auditExportFileDialog.currentFile, pendingAuditJson);
                                    if (err !== "") showError("Save failed: " + err);
                                    else showInfo("Audit log saved");
                                }
                            }

                            Text { text: qsTr("Ferric never reads browser cookie databases. Audit payloads contain download/torrent events only - no passkeys, no proxy credentials."); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width }

                            Text { text: qsTr("Integration"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4; color: session.connected ? Theme.success : Theme.danger }
                                Text { anchors.verticalCenter: parent.verticalCenter; color: session.connected ? Theme.success : Theme.danger; font.pixelSize: Theme.fontSizeSmall
                                    text: session.connected ? "Daemon connected \u2014 port " + session.port : "Daemon disconnected" }
                            }

                            Row { spacing: Theme.sp2; height: Theme.rowH
                                Rectangle { width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                                    property bool checked: (prefsData["portable_mode"] || "0") === "1"
                                    color: checked ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                                    Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                                    MouseArea { anchors.fill: parent; onClicked: savePref("portable_mode", parent.checked ? "0" : "1") }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Portable mode (data dir next to binary)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            }

                            // ── FQT-104: WebUI \u2014 NOT IMPLEMENTED ─────────
                            Text { text: qsTr("Web UI (remote admin)"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
                            Text { text: qsTr("Not implemented. Settings exist but are inert \u2014 no server code was written. Do not enable, has no effect."); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width }
                        }
                        } // Advanced tab Item
                    }
                }

                // Close button
                Row {
                    id: prefsFooter
                    anchors.horizontalCenter: parent.horizontalCenter
                    Rectangle {
                        width: Theme.btnM; height: Theme.controlH; radius: Theme.radius
                        color: prefsCloseArea.containsMouse ? Theme.border : Theme.surfaceAlt
                        Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
                        MouseArea { id: prefsCloseArea; anchors.fill: parent; hoverEnabled: true; onClicked: prefsDialog.visible = false }
                    }
                }
            }
        }
}
