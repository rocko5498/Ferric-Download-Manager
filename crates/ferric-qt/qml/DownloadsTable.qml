import QtQuick
import QtQuick.Controls

// Downloads table \u2014 extracted from main.qml (Phase 2 split #19)
Flickable {
    id: downloadHFlick

    anchors.fill: parent
    anchors.margins: 8
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
            height: 22
            Text { width: 28; text: qsTr("#"); color: Theme.textDim; font.pixelSize: 11 }
            Text { width: 24; text: qsTr(""); color: Theme.textDim; font.pixelSize: 11 }
            Repeater {
                model: downloadHFlick.columns
                Text {
                    width: modelData.width
                    text: modelData.title
                    color: Theme.textDim
                    font.pixelSize: 11
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
                         !downloadHFlick.findMatches(model.name || "", model.urlHost || "", model.savePath || "", model.description || "")) ? 0 : 22
                visible: (downloadHFlick.categoryFilter === "" || model.category === downloadHFlick.categoryFilter) &&
                         downloadHFlick.hiddenIds[model.id] !== true &&
                         downloadHFlick.queueMatches(model.queue) &&
                         downloadHFlick.findMatches(model.name || "", model.urlHost || "", model.savePath || "", model.description || "")
                color: downloadHFlick.selectedDownloads.some(function(s) { return s.id === model.id; }) ? Theme.surfaceAlt : "transparent"

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
                        case "queue": return rowRoot.rQueue
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
                    Text { width: 28; text: (index + 1); color: Theme.textDim; font.pixelSize: 11 }
                    Rectangle {
                        width: 10; height: 10; radius: 5
                        color: rowRoot.rStatus === "downloading" ? Theme.success
                            : rowRoot.rStatus === "error" ? Theme.danger
                            : rowRoot.rStatus === "paused" ? Theme.warning
                            : Theme.textDim
                    }
                    Item { width: 14; height: 1 }

                    Repeater {
                        model: downloadHFlick.columns
                        delegate: Item {
                            id: cell
                            width: modelData.width
                            height: 22
                            property string role: modelData.role
                            property string val: rowRoot.roleValue(role)

                            // Loader: only the progress column pays for the bar objects;
                            // plain cells stay one Text each.
                            Loader {
                                active: cell.role === "progress"
                                anchors.verticalCenter: parent.verticalCenter
                                sourceComponent: Row {
                                    spacing: 5
                                    property real pct: {
                                        var n = parseFloat(cell.val);
                                        return isNaN(n) ? 0 : Math.max(0, Math.min(100, n));
                                    }
                                    Rectangle {
                                        width: 42; height: 5; radius: 3
                                        anchors.verticalCenter: parent.verticalCenter
                                        color: Theme.border
                                        visible: cell.val && cell.val !== "-"
                                        Rectangle {
                                            width: parent.width * (parent.parent.pct / 100)
                                            height: parent.height; radius: parent.radius
                                            color: rowRoot.rStatus === "completed" ? Theme.textDim : Theme.accent
                                            Behavior on width { NumberAnimation { duration: 300; easing.type: Easing.Linear } }
                                        }
                                    }
                                    Text {
                                        anchors.verticalCenter: parent.verticalCenter
                                        color: Theme.textDim; font.pixelSize: 11
                                        text: cell.val
                                textFormat: Text.PlainText
                                    }
                                }
                            }
                            Text {
                                visible: cell.role !== "progress"
                                anchors.verticalCenter: parent.verticalCenter
                                width: parent.width
                                elide: Text.ElideRight
                                color: cell.role === "error" ? Theme.danger
                                    : (cell.role === "savePath" || cell.role === "urlHost") ? Theme.textDim
                                    : Theme.text
                                font.pixelSize: 12
                                text: cell.val
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
        font.pixelSize: 13
        visible: !downloadHFlick.lastOk && downloadHFlick.lastError !== ""
        color: Theme.danger
        text: qsTr("Cannot load downloads") + "\n" + downloadHFlick.lastError
    }

    Text {
        anchors.centerIn: parent
        width: parent.width - 48
        z: 2
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        font.pixelSize: 13
        visible: downloadHFlick.downloadModel && downloadHFlick.downloadModel.count === 0 && downloadHFlick.lastOk
        color: Theme.textDim
        text: qsTr("No downloads yet \u2014 Add URL or capture from the browser")
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
