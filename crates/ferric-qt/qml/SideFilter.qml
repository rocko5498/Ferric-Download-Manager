import QtQuick
import QtQuick.Controls as QQC

// SideFilter panel \u2014 extracted from main.qml (Phase 2 split #16)
Rectangle {
    id: sideFilter

    width: Theme.sidebarW
    height: parent.height
    color: Theme.bg

    property var categories: []
    property string categoryFilter: ""
    property var queues: []
    property string queueFilter: ""
    property var tags: []
    property bool torrentsVisible: false
    property string tagFilter: ""
    property string statusFilter: ""
    property var onCategoryChanged: function(filter) {}
    property var onQueueChanged: function(filter) {}
    property var onTagChanged: function(filter) {}
    property var onStatusChanged: function(filter) {}
    property var onEditQueue: function(queue) {}
    property var onAddCategory: function() {}
    property var onNewQueue: function() {}
    property var onQueueAction: function(queueId, action) {}
    property var onDeleteQueue: function(queue) {}
    readonly property string defaultQueueId: "00000000-0000-0000-0000-000000000001"

    function categoryIcon(n) {
        var c = (n || "").toLowerCase()
        if (c === "compressed") return "ft-archive"
        if (c === "documents") return "ft-document"
        if (c === "music") return "ft-music"
        if (c === "programs") return "ft-program"
        if (c === "video") return "ft-video"
        return "ft-other"
    }
    function statusIcon(s) {
        return s === "Downloading" ? "sb-downloading" : s === "Paused" ? "sb-paused" : s === "Completed" ? "sb-completed" : s === "Error" ? "sb-error" : ""
    }

    // One row of the sidebar: icon or dot, label, and a clear orange selected state.
    component SideItem: Rectangle {
        id: it
        property string label: ""
        property string icon: ""
        property bool selected: false
        property bool dot: false
        property color dotColor: Theme.textDim
        signal clicked(var mouse)
        signal doubleClicked()
        width: sideFilter.width - 2 * Theme.sp2; height: Theme.controlH; radius: Theme.radius
        color: selected ? Theme.accentSoft : (itArea.containsMouse ? Theme.rowHover : "transparent")
        Rectangle { opacity: it.selected ? 1 : 0; Behavior on opacity { enabled: Theme.motion; NumberAnimation { duration: Theme.animMs } } visible: opacity > 0; width: 3; height: parent.height - 2 * Theme.sp1; radius: Theme.radius; color: Theme.accent; anchors.verticalCenter: parent.verticalCenter }
        Row {
            anchors.left: parent.left; anchors.leftMargin: Theme.sp3; anchors.verticalCenter: parent.verticalCenter
            spacing: Theme.sp2
            Image { visible: it.icon !== ""; anchors.verticalCenter: parent.verticalCenter; width: 18; height: 18; sourceSize: Qt.size(72, 72); smooth: true; opacity: it.selected ? 1 : 0.85; source: it.icon !== "" ? "qrc:/icons/" + it.icon + ".png" : "" }
            Rectangle { visible: it.dot; anchors.verticalCenter: parent.verticalCenter; width: Theme.dotSize; height: Theme.dotSize; radius: Theme.dotSize / 2; color: it.dotColor }
            Text { anchors.verticalCenter: parent.verticalCenter; text: it.label; color: it.selected ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeNormal; font.weight: it.selected ? Font.DemiBold : Font.Normal }
        }
        MouseArea {
            id: itArea; anchors.fill: parent; hoverEnabled: true; acceptedButtons: Qt.LeftButton | Qt.RightButton
            onClicked: function(mouse) { it.clicked(mouse) }
            onDoubleClicked: it.doubleClicked()
        }
    }

    Flickable {
        anchors.fill: parent
        contentHeight: filterColumn.height + 16
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        QQC.ScrollBar.vertical: QQC.ScrollBar {}

        Column {
            id: filterColumn
            x: 8
            y: 8
            width: sideFilter.width - 16
            spacing: Theme.sp1

        // Brand
        Row {
            spacing: Theme.sp2
            height: Theme.sp5 + Theme.sp2
            Image { anchors.verticalCenter: parent.verticalCenter; width: Theme.sp5; height: Theme.sp5; sourceSize: Qt.size(96, 96); smooth: true; source: "qrc:/icons/ferric-logo-96.png" }
            Column {
                anchors.verticalCenter: parent.verticalCenter
                Text { text: "Ferric"; color: Theme.text; font.pixelSize: Theme.fontSizeMedium; font.weight: Font.Bold }
                Text { text: qsTr("Download Manager"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            }
        }
        Item { width: 1; height: Theme.sp2 }

        // Categories
        Row {
            spacing: Theme.sp1
            height: Theme.sp4
            Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("CATEGORIES"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; font.weight: Font.DemiBold; font.letterSpacing: 1 }
            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: Theme.sp4; height: Theme.sp4; radius: Theme.radius; color: addCatArea.containsMouse ? Theme.surfaceAlt : "transparent"
                Text { anchors.centerIn: parent; text: qsTr("+"); color: Theme.textDim; font.pixelSize: Theme.fontSizeMedium }
                MouseArea { id: addCatArea; anchors.fill: parent; hoverEnabled: true; onClicked: sideFilter.onAddCategory() }
            }
        }

        SideItem { label: qsTr("All downloads"); icon: "sb-all"; selected: sideFilter.categoryFilter === ""; onClicked: sideFilter.onCategoryChanged("") }

        Repeater {
            model: sideFilter.categories
            SideItem { label: modelData.name; icon: sideFilter.categoryIcon(modelData.name); selected: sideFilter.categoryFilter === modelData.name; onClicked: sideFilter.onCategoryChanged(modelData.name) }
        }

        // Queues
        Item { width: 1; height: Theme.sp2 }
        Row {
            spacing: Theme.sp1
            height: Theme.sp4
            Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("QUEUES"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; font.weight: Font.DemiBold; font.letterSpacing: 1 }
            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: Theme.sp4; height: Theme.sp4; radius: Theme.radius; color: addQueueArea.containsMouse ? Theme.surfaceAlt : "transparent"
                Text { anchors.centerIn: parent; text: qsTr("+"); color: Theme.textDim; font.pixelSize: Theme.fontSizeMedium }
                MouseArea { id: addQueueArea; anchors.fill: parent; hoverEnabled: true; onClicked: sideFilter.onNewQueue() }
            }
        }
        Text {
            text: qsTr("No queues yet")
            color: Theme.textDim
            font.pixelSize: Theme.fontSizeSmall
            visible: sideFilter.queues.length === 0
            anchors.left: parent.left
            anchors.leftMargin: Theme.sp1
        }

        Repeater {
            model: sideFilter.queues
            SideItem {
                label: modelData.name; dot: true; dotColor: modelData.enabled ? Theme.success : Theme.textDim
                selected: sideFilter.queueFilter === modelData.id
                onClicked: function(mouse) {
                    if (mouse.button === Qt.RightButton) {
                        queueMenu.queue = modelData;
                        queueMenu.popup();
                    } else {
                        sideFilter.onQueueChanged(sideFilter.queueFilter === modelData.id ? "" : modelData.id);
                    }
                }
                onDoubleClicked: sideFilter.onEditQueue(modelData)
            }
        }

        // Status
        Item { width: 1; height: Theme.sp2 }
        Text { text: qsTr("STATUS"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; font.weight: Font.DemiBold; font.letterSpacing: 1 }

        SideItem { label: qsTr("All"); icon: "sb-all"; selected: sideFilter.statusFilter === ""; onClicked: sideFilter.onStatusChanged("") }

        Repeater {
            model: sideFilter.torrentsVisible
                ? ["Downloading", "Paused", "Completed", "Error", "Staged", "Checking", "Ready to start"]
                : ["Downloading", "Paused", "Completed", "Error"]
            SideItem { label: modelData; icon: sideFilter.statusIcon(modelData); selected: sideFilter.statusFilter === modelData; onClicked: sideFilter.onStatusChanged(sideFilter.statusFilter === modelData ? "" : modelData) }
        }

        // Tags (torrent tags only)
        Item { visible: sideFilter.torrentsVisible; width: 1; height: 8 }
        Text { visible: sideFilter.torrentsVisible; text: qsTr("TAGS"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }

        Rectangle {
            visible: sideFilter.torrentsVisible
            width: sideFilter.width - 16; height: Theme.rowH; radius: Theme.radius
            color: sideFilter.tagFilter === "" ? Theme.surfaceAlt : "transparent"
            Text { anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter; text: qsTr("All tags"); color: sideFilter.tagFilter === "" ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: sideFilter.onTagChanged("") }
        }

        Repeater {
            model: sideFilter.torrentsVisible ? sideFilter.tags : []
            Rectangle {
                width: sideFilter.width - 16; height: Theme.rowH; radius: Theme.radius
                color: sideFilter.tagFilter === modelData ? Theme.surfaceAlt : "transparent"
                Text { anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter; text: modelData; color: sideFilter.tagFilter === modelData ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: sideFilter.onTagChanged(sideFilter.tagFilter === modelData ? "" : modelData) }
            }
        }
        }
    }

    // IDM's queue commands: Start queue / Stop queue (Downloads menu) and the Scheduler's New queue / Delete.
    ThemedMenu {
        id: queueMenu
        property var queue: null
        ThemedMenuItem { text: qsTr("Start queue"); onTriggered: sideFilter.onQueueAction(queueMenu.queue.id, "start") }
        ThemedMenuItem { text: qsTr("Stop queue"); onTriggered: sideFilter.onQueueAction(queueMenu.queue.id, "stop") }
        ThemedMenuSeparator {}
        ThemedMenuItem { text: qsTr("Edit queue..."); onTriggered: sideFilter.onEditQueue(queueMenu.queue) }
        ThemedMenuItem {
            text: qsTr("Delete queue")
            enabled: queueMenu.queue !== null && queueMenu.queue.id !== sideFilter.defaultQueueId
            onTriggered: sideFilter.onDeleteQueue(queueMenu.queue)
        }
    }
}
