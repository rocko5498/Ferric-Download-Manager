import QtQuick
import QtQuick.Controls as QQC

// SideFilter panel \u2014 extracted from main.qml (Phase 2 split #16)
Rectangle {
    id: sideFilter

    width: 180
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
            spacing: 6

        // Categories
        Row {
            spacing: 4
            Text { text: qsTr("CATEGORIES"); color: Theme.textDim; font.pixelSize: 10 }
            Rectangle {
                width: 14; height: 14; radius: 3; color: addCatArea.containsMouse ? Theme.surfaceAlt : "transparent"
                Text { anchors.centerIn: parent; text: qsTr("+"); color: Theme.textDim; font.pixelSize: 12 }
                MouseArea { id: addCatArea; anchors.fill: parent; hoverEnabled: true; onClicked: sideFilter.onAddCategory() }
            }
        }

        Rectangle {
            width: parent.width - 16; height: 22; radius: 3
            color: sideFilter.categoryFilter === "" ? Theme.surfaceAlt : "transparent"
            Text { anchors.left: parent.left; text: qsTr("All downloads"); color: sideFilter.categoryFilter === "" ? Theme.text : Theme.textDim; font.pixelSize: 11 }
            MouseArea { anchors.fill: parent; onClicked: sideFilter.onCategoryChanged("") }
        }

        Repeater {
            model: sideFilter.categories
            Rectangle {
                width: sideFilter.width - 16; height: 22; radius: 3
                color: sideFilter.categoryFilter === modelData.name ? Theme.surfaceAlt : "transparent"
                Row {
                    anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
                    spacing: 4
                    Text { text: modelData.name; color: sideFilter.categoryFilter === modelData.name ? Theme.text : Theme.textDim; font.pixelSize: 11 }
                    Rectangle {
                        visible: modelData.pattern && modelData.pattern.length > 0
                        width: patternLabel.width + 6; height: 14; radius: 3
                        color: Theme.surface; border.color: Theme.surfaceAlt; border.width: 1
                        Text { id: patternLabel; anchors.centerIn: parent; text: modelData.pattern || ""; color: Theme.textDim; font.pixelSize: 8 }
                    }
                }
                MouseArea { anchors.fill: parent; onClicked: sideFilter.onCategoryChanged(modelData.name) }
            }
        }

        // Queues
        Item { width: 1; height: 8 }
        Row {
            spacing: 6
            Text { text: qsTr("QUEUES"); color: Theme.textDim; font.pixelSize: 10 }
            Text {
                text: "+"; color: Theme.textDim; font.pixelSize: 12
                MouseArea { anchors.fill: parent; anchors.margins: -4; cursorShape: Qt.PointingHandCursor; onClicked: sideFilter.onNewQueue() }
            }
        }
        Text {
            text: qsTr("No queues yet")
            color: Theme.textDim
            font.pixelSize: 10
            visible: sideFilter.queues.length === 0
            anchors.left: parent.left
            anchors.leftMargin: 4
        }

        Repeater {
            model: sideFilter.queues
            Rectangle {
                width: sideFilter.width - 16; height: 22; radius: 3
                color: sideFilter.queueFilter === modelData.id ? Theme.surfaceAlt : "transparent"
                Row {
                    anchors.left: parent.left
                    spacing: 4
                    Rectangle {
                        width: 6; height: 6; radius: 3
                        color: modelData.enabled ? Theme.success : Theme.textDim
                    }
                    Text { text: modelData.name; color: sideFilter.queueFilter === modelData.id ? Theme.text : Theme.textDim; font.pixelSize: 11 }
                }
                MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onDoubleClicked: sideFilter.onEditQueue(modelData)
                    onClicked: function(mouse) {
                        if (mouse.button === Qt.RightButton) {
                            queueMenu.queue = modelData;
                            queueMenu.popup();
                        } else {
                            sideFilter.onQueueChanged(sideFilter.queueFilter === modelData.id ? "" : modelData.id);
                        }
                    }
                }
            }
        }

        // Status
        Item { width: 1; height: 8 }
        Text { text: qsTr("STATUS"); color: Theme.textDim; font.pixelSize: 10 }

        Rectangle {
            width: sideFilter.width - 16; height: 22; radius: 3
            color: sideFilter.statusFilter === "" ? Theme.surfaceAlt : "transparent"
            Text { anchors.left: parent.left; text: qsTr("All"); color: sideFilter.statusFilter === "" ? Theme.text : Theme.textDim; font.pixelSize: 11 }
            MouseArea { anchors.fill: parent; onClicked: sideFilter.onStatusChanged("") }
        }

        Repeater {
            model: sideFilter.torrentsVisible
                ? ["Downloading", "Paused", "Completed", "Error", "Staged", "Checking", "Ready to start"]
                : ["Downloading", "Paused", "Completed", "Error"]
            Rectangle {
                width: sideFilter.width - 16; height: 22; radius: 3
                color: sideFilter.statusFilter === modelData ? Theme.surfaceAlt : "transparent"
                Text { anchors.left: parent.left; text: modelData; color: sideFilter.statusFilter === modelData ? Theme.text : Theme.textDim; font.pixelSize: 11 }
                MouseArea { anchors.fill: parent; onClicked: sideFilter.onStatusChanged(sideFilter.statusFilter === modelData ? "" : modelData) }
            }
        }

        // Tags (torrent tags only)
        Item { visible: sideFilter.torrentsVisible; width: 1; height: 8 }
        Text { visible: sideFilter.torrentsVisible; text: qsTr("TAGS"); color: Theme.textDim; font.pixelSize: 10 }

        Rectangle {
            visible: sideFilter.torrentsVisible
            width: sideFilter.width - 16; height: 22; radius: 3
            color: sideFilter.tagFilter === "" ? Theme.surfaceAlt : "transparent"
            Text { anchors.left: parent.left; text: qsTr("All tags"); color: sideFilter.tagFilter === "" ? Theme.text : Theme.textDim; font.pixelSize: 11 }
            MouseArea { anchors.fill: parent; onClicked: sideFilter.onTagChanged("") }
        }

        Repeater {
            model: sideFilter.torrentsVisible ? sideFilter.tags : []
            Rectangle {
                width: sideFilter.width - 16; height: 22; radius: 3
                color: sideFilter.tagFilter === modelData ? Theme.surfaceAlt : "transparent"
                Text { anchors.left: parent.left; text: modelData; color: sideFilter.tagFilter === modelData ? Theme.text : Theme.textDim; font.pixelSize: 11 }
                MouseArea { anchors.fill: parent; onClicked: sideFilter.onTagChanged(sideFilter.tagFilter === modelData ? "" : modelData) }
            }
        }
        }
    }

    // IDM's queue commands: Start queue / Stop queue (Downloads menu) and the Scheduler's New queue / Delete.
    QQC.Menu {
        id: queueMenu
        property var queue: null
        QQC.MenuItem { text: qsTr("Start queue"); onTriggered: sideFilter.onQueueAction(queueMenu.queue.id, "start") }
        QQC.MenuItem { text: qsTr("Stop queue"); onTriggered: sideFilter.onQueueAction(queueMenu.queue.id, "stop") }
        QQC.MenuSeparator {}
        QQC.MenuItem { text: qsTr("Edit queue..."); onTriggered: sideFilter.onEditQueue(queueMenu.queue) }
        QQC.MenuItem {
            text: qsTr("Delete queue")
            enabled: queueMenu.queue !== null && queueMenu.queue.id !== sideFilter.defaultQueueId
            onTriggered: sideFilter.onDeleteQueue(queueMenu.queue)
        }
    }
}
