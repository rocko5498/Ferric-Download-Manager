import QtQuick
import QtQuick.Controls

// Downloads table \u2014 extracted from main.qml (Phase 2 split #19)
Flickable {
    id: downloadHFlick

    anchors.fill: parent
    anchors.margins: Theme.sp2
    contentWidth: Math.max(width, tableWidth)
    contentHeight: height
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    property var downloadModel: null
    property var columns: []
    property real tableWidth: 0
    property var selectedDownloads: []
    property string categoryFilter: ""
    // Captures still waiting for the Download File Info dialog are not part of the list yet.
    property var hiddenIds: ({})
    property string queueFilter: ""
    property var queues: []
    // IDM's Find (Ctrl-F): keep only rows whose name, host or folder contains the text.
    property string findText: ""
    // IDM's Find options: what to search ("name", "description" or "site"), match case, whole string.
    property string findScope: "name"
    property bool findMatchCase: false
    property bool findWhole: false

    function findMatches(name, host, path, description) {
        if (findText === "") return true;
        var hay = findScope === "description" ? (description || "")
                : findScope === "site" ? (host + " " + path)
                : name;
        var needle = findText;
        if (!findMatchCase) { hay = hay.toLowerCase(); needle = needle.toLowerCase(); }
        return findWhole ? hay === needle : hay.indexOf(needle) >= 0;
    }

    // The model's `queue` is the first 8 chars of the queue id ("" = default queue), while the
    // sidebar filter holds the full id, so the queue filter never matched and showed nothing.
    // file-type badge for a row: by category, then by extension
    function fileIcon(category, name) {
        var c = (category || "").toLowerCase()
        if (c === "video") return "ft-video"
        if (c === "compressed") return "ft-archive"
        if (c === "documents") return "ft-document"
        if (c === "music") return "ft-music"
        if (c === "programs") return "ft-program"
        var e = (name || "").split(".").pop().toLowerCase()
        if (["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"].indexOf(e) >= 0) return "ft-image"
        return "ft-other"
    }
    readonly property int leadW: 28 + Theme.sp2   // row number + file icon cell + gap: header and rows share it
    function queueLabel(id) {
        for (var i = 0; i < queues.length; i++) if (queues[i].id === id) return queues[i].name
        return ""
    }
    function queueMatches(rowQueue) {
        if (queueFilter === "") return true;
        return (rowQueue === "" ? "00000000" : rowQueue) === queueFilter.substring(0, 8);
    }
    signal openProgress(string id)
    property var onToggleSelection: function(id, status, name, savePath, urlHost, additive) {}
    property var onShowProperties: function(id, status, name, savePath, urlHost) {}
    property var bus: null
    property var requestBus: null
    property string lastError: ""
    property bool lastOk: true

    Column {
        width: downloadHFlick.contentWidth

        Row {
            id: downloadHeaderRow
            height: Theme.rowH
            Text { width: 28; height: parent.height; verticalAlignment: Text.AlignVCenter; text: qsTr("#"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Item { width: downloadHFlick.leadW; height: parent.height }
            Repeater {
                model: downloadHFlick.columns
                Text {
                    width: modelData.width
                    height: Theme.rowH
                    verticalAlignment: Text.AlignVCenter
                    font.weight: Font.DemiBold
                    text: modelData.title
                    color: Theme.textDim
                    font.pixelSize: Theme.fontSizeSmall
                    elide: Text.ElideRight
                }
            }
        }

        ListView {
            id: downloadListView
            width: downloadHFlick.contentWidth
            height: downloadHFlick.height - downloadHeaderRow.height
            clip: true
            cacheBuffer: 200
            model: downloadHFlick.downloadModel
            delegate: Rectangle {
                id: rowRoot
                width: downloadHFlick.contentWidth
                height: ((downloadHFlick.categoryFilter !== "" && model.category !== downloadHFlick.categoryFilter) ||
                         downloadHFlick.hiddenIds[model.id] === true ||
                         !downloadHFlick.queueMatches(model.queue) ||
                         !downloadHFlick.findMatches(model.name || "", model.urlHost || "", model.savePath || "", model.description || "")) ? 0 : Theme.listRowH
                visible: (downloadHFlick.categoryFilter === "" || model.category === downloadHFlick.categoryFilter) &&
                         downloadHFlick.hiddenIds[model.id] !== true &&
                         downloadHFlick.queueMatches(model.queue) &&
                         downloadHFlick.findMatches(model.name || "", model.urlHost || "", model.savePath || "", model.description || "")
                readonly property bool isSelected: downloadHFlick.selectedDownloads.some(function(s) { return s.id === model.id; })
                color: isSelected ? Theme.accentSoft : (rowHover.hovered ? Theme.rowHover : "transparent")
                HoverHandler { id: rowHover }
                Rectangle { visible: rowRoot.isSelected; width: 3; height: parent.height - 2 * Theme.sp1; radius: Theme.radius; color: Theme.accent; anchors.verticalCenter: parent.verticalCenter }

                // Captured once per row so the Repeater below (which shadows the
                // ListView's `model` context property with its own) can still reach
                // the row's data — this is what makes column REORDER safe: cells are
                // looked up by role name, not by a hardcoded positional index.
                property string rName: model.name || ""
                property string rSize: model.size || ""
                property string rDone: model.done || ""
                property string rProgress: model.progress || ""
                property string rStatus: model.status || ""
                property string rSpeed: model.speed || ""
                property string rEta: model.eta || ""
                property string rQueue: model.queue || ""
                property string rCategory: model.category || ""
                property string rAdded: model.added || ""
                property string rCompleted: model.completed || ""
                property string rSavePath: model.savePath || ""
                property string rUrlHost: model.urlHost || ""
                property string rError: model.error || ""

                function roleValue(role) {
                    switch (role) {
                        case "name": return rowRoot.rName
                        case "size": return rowRoot.rSize
                        case "done": return rowRoot.rDone
                        case "progress": return rowRoot.rProgress
                        case "status": return rowRoot.rStatus
                        case "speed": return rowRoot.rSpeed
                        case "eta": return rowRoot.rEta
                        case "queue": return downloadHFlick.queueLabel(rowRoot.rQueue)
                        case "category": return rowRoot.rCategory
                        case "added": return rowRoot.rAdded
                        case "completed": return rowRoot.rCompleted
                        case "savePath": return rowRoot.rSavePath
                        case "urlHost": return rowRoot.rUrlHost
                        case "error": return rowRoot.rError
                        default: return "-"
                    }
                }

                Row {
                    anchors.fill: parent
                    Text { width: 28; height: parent.height; verticalAlignment: Text.AlignVCenter; text: (index + 1); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Item {
                        width: 28; height: parent.height
                        Image {
                            anchors.centerIn: parent
                            width: 24; height: 24; sourceSize: Qt.size(72, 72); smooth: true
                            source: "qrc:/icons/" + downloadHFlick.fileIcon(rowRoot.rCategory, rowRoot.rName) + ".png"
                        }
                    }
                    Item { width: Theme.sp2; height: 1 }

                    Repeater {
                        model: downloadHFlick.columns
                        delegate: Item {
                            id: cell
                            width: modelData.width
                            height: Theme.listRowH
                            property string role: modelData.role
                            property string val: rowRoot.roleValue(role)
                            property string st: rowRoot.rStatus
                            property real pct: { var n = parseFloat(val); return isNaN(n) ? 0 : Math.max(0, Math.min(100, n)) }

                            // Name: the file name over the site it comes from
                            Column {
                                visible: cell.role === "name"
                                anchors.verticalCenter: parent.verticalCenter
                                width: parent.width - Theme.sp2
                                spacing: 1
                                Text { width: parent.width; elide: Text.ElideRight; text: cell.val; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.weight: Font.Medium; textFormat: Text.PlainText }
                                Text { width: parent.width; elide: Text.ElideRight; visible: rowRoot.rUrlHost !== ""; text: rowRoot.rUrlHost; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; textFormat: Text.PlainText }
                            }

                            // Status: a coloured pill
                            Rectangle {
                                visible: cell.role === "status"
                                anchors.verticalCenter: parent.verticalCenter
                                height: Theme.sp4; width: pillText.implicitWidth + 2 * Theme.sp3; radius: height / 2
                                color: cell.st === "completed" ? Theme.successSoft : cell.st === "downloading" ? Theme.accentSoft
                                     : cell.st === "paused" ? Theme.warningSoft : cell.st === "error" ? Theme.dangerSoft : Theme.surfaceAlt
                                Text {
                                    id: pillText
                                    anchors.centerIn: parent
                                    text: cell.val.charAt(0).toUpperCase() + cell.val.slice(1)
                                    font.pixelSize: Theme.fontSizeSmall; font.weight: Font.DemiBold
                                    color: cell.st === "completed" ? Theme.success : cell.st === "downloading" ? Theme.accent
                                         : cell.st === "paused" ? Theme.warningText : cell.st === "error" ? Theme.danger : Theme.textDim
                                }
                            }

                            // Progress: a bar as wide as its column, and the percentage
                            Row {
                                visible: cell.role === "progress" && cell.val !== "" && cell.val !== "-"
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: Theme.sp2
                                Rectangle {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: Math.max(Theme.sp5, cell.width - 3 * Theme.sp4 - Theme.sp2); height: 6; radius: 3
                                    color: Theme.border
                                    Rectangle {
                                        width: parent.width * (cell.pct / 100); height: parent.height; radius: parent.radius
                                        color: cell.st === "completed" ? Theme.success : cell.st === "error" ? Theme.danger : Theme.accent
                                        Behavior on width { NumberAnimation { duration: 300; easing.type: Easing.Linear } }
                                    }
                                }
                                Text { anchors.verticalCenter: parent.verticalCenter; text: cell.val; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; textFormat: Text.PlainText }
                            }

                            // Everything else: plain text, dimmer when it is secondary information
                            Text {
                                visible: cell.role !== "name" && cell.role !== "status" && cell.role !== "progress"
                                anchors.verticalCenter: parent.verticalCenter
                                width: parent.width - Theme.sp2
                                elide: Text.ElideRight
                                text: cell.val
                                font.pixelSize: Theme.fontSizeNormal
                                color: cell.role === "error" ? Theme.danger
                                     : (cell.role === "size" || cell.role === "speed" || cell.role === "eta") ? Theme.text : Theme.textDim
                                textFormat: Text.PlainText
                            }
                        }
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onDoubleClicked: function(mouse) {
                        if (mouse.button === Qt.LeftButton) downloadHFlick.openProgress(model.id)
                    }
                    onClicked: function(mouse) {
                        if (mouse.button === Qt.RightButton) {
                            downloadContextMenu.rowId = model.id
                            downloadContextMenu.rowName = model.name
                            downloadContextMenu.rowSavePath = model.savePath
                            downloadContextMenu.rowUrlHost = model.urlHost
                            downloadContextMenu.popup()
                        } else {
                            downloadHFlick.onToggleSelection(
                                model.id, model.status, model.name, model.savePath, model.urlHost,
                                (mouse.modifiers & Qt.ControlModifier) !== 0)
                        }
                    }
                }
            }
        }
    }

    // Empty/error state
    Text {
        anchors.centerIn: parent
        width: parent.width - 48
        z: 2
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        font.pixelSize: Theme.fontSizeNormal
        visible: !downloadHFlick.lastOk && downloadHFlick.lastError !== ""
        color: Theme.danger
        text: qsTr("Cannot load downloads") + "\n" + downloadHFlick.lastError
    }

    Column {
        anchors.centerIn: parent
        z: 2
        spacing: Theme.sp2
        visible: downloadHFlick.downloadModel && downloadHFlick.downloadModel.count === 0 && downloadHFlick.lastOk
        Image { anchors.horizontalCenter: parent.horizontalCenter; width: 96; height: 96; sourceSize: Qt.size(192, 192); smooth: true; source: "qrc:/icons/empty-downloads.png" }
        Text { anchors.horizontalCenter: parent.horizontalCenter; text: qsTr("No downloads yet"); color: Theme.text; font.pixelSize: Theme.fontSizeMedium; font.weight: Font.DemiBold }
        Text { anchors.horizontalCenter: parent.horizontalCenter; text: qsTr("Click Add URL, or start a download in your browser."); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
    }

    Menu {
        id: downloadContextMenu
        property string rowId: ""
        property string rowName: ""
        property string rowSavePath: ""
        property string rowUrlHost: ""
        property string rowUrl: ""
        MenuItem {
            text: qsTr("Properties")
            onTriggered: downloadHFlick.onShowProperties(downloadContextMenu.rowId, "", downloadContextMenu.rowName, downloadContextMenu.rowSavePath, downloadContextMenu.rowUrlHost)
        }
        MenuItem {
            text: qsTr("Open")
            enabled: downloadHFlick.bus !== null
            onTriggered: {
                if (downloadHFlick.bus)
                    downloadHFlick.bus.open_file(downloadContextMenu.rowSavePath + "/" + downloadContextMenu.rowName)
            }
        }
        MenuItem {
            text: qsTr("Open Folder")
            enabled: downloadHFlick.bus !== null
            onTriggered: {
                if (downloadHFlick.bus)
                    downloadHFlick.bus.open_folder(downloadContextMenu.rowSavePath)
            }
        }
        MenuSeparator {}
        MenuItem {
            text: qsTr("Copy URL")
            onTriggered: {
                clipboardInput.text = downloadContextMenu.rowUrlHost
                clipboardInput.selectAll(); clipboardInput.copy()
            }
        }
        MenuSeparator {}
        MenuItem {
            text: qsTr("Pause")
            enabled: downloadHFlick.bus !== null
            onTriggered: { if (downloadHFlick.requestBus) downloadHFlick.requestBus("download_action", { id: downloadContextMenu.rowId, action: "pause" }, function() {}) }
        }
        MenuItem {
            text: qsTr("Resume")
            enabled: downloadHFlick.bus !== null
            onTriggered: { if (downloadHFlick.requestBus) downloadHFlick.requestBus("download_action", { id: downloadContextMenu.rowId, action: "resume" }, function() {}) }
        }
        MenuItem {
            text: qsTr("Download Now")
            enabled: downloadHFlick.bus !== null
            onTriggered: { if (downloadHFlick.requestBus) downloadHFlick.requestBus("download_action", { id: downloadContextMenu.rowId, action: "start-now" }, function() {}) }
        }
        Menu {
            title: qsTr("Move to queue")
            Repeater {
                model: downloadHFlick.queues
                MenuItem {
                    text: modelData.name
                    onTriggered: {
                        if (downloadHFlick.requestBus)
                            downloadHFlick.requestBus("move_download_to_queue", { id: downloadContextMenu.rowId, queue_id: modelData.id }, function() { downloadHFlick.downloadModel.refresh() })
                    }
                }
            }
        }
        MenuItem {
            text: qsTr("Redownload")
            enabled: downloadHFlick.bus !== null
            onTriggered: { if (downloadHFlick.requestBus) downloadHFlick.requestBus("download_action", { id: downloadContextMenu.rowId, action: "redownload" }, function() {}) }
        }
        MenuSeparator {}
        MenuItem {
            text: qsTr("Delete (keep file)")
            onTriggered: {
                if (downloadHFlick.requestBus) {
                    downloadHFlick.requestBus("delete_download", { id: downloadContextMenu.rowId, with_file: false }, function() {
                        downloadHFlick.downloadModel.refresh()
                    })
                }
            }
        }
        MenuItem {
            text: qsTr("Delete + Remove File")
            onTriggered: {
                if (downloadHFlick.requestBus) {
                    downloadHFlick.requestBus("delete_download", { id: downloadContextMenu.rowId, with_file: true }, function() {
                        downloadHFlick.downloadModel.refresh()
                    })
                }
            }
        }
    }
}
