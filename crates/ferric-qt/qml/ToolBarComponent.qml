import QtQuick

Rectangle {
    id: toolBar

    property bool small: false
    height: small ? 38 : 52
    color: Theme.surface
    border.color: Theme.border
    border.width: 1

    property int workspaceIndex: 0
    property var selectedDownloads: []
    property var selectedTorrents: []
    property bool canPause: false
    property bool canResume: false
    property bool torrentCanStart: false
    property bool torrentCanPause: false
    property bool torrentCanResume: false
    property var onAction: function(actionId) {}

    // btnEnabled, not enabled: shadowing Item.enabled leaves the real property
    // true while the shadow carries state — anything reading Item semantics breaks.
    component ToolBtn: Rectangle {
        id: btn
        property string actionId: ""
        property string label: ""
        property string icon: ""
        property bool btnEnabled: true
        property var onAction: function(id) {}
        // Oxygen icon name (LGPL, see icons/LICENSE-NOTE.md); the glyph stays as fallback.
        property string iconName: ""
        readonly property string iconFile: iconName

        width: toolBar.small ? 34 : 60; height: toolBar.small ? 32 : 44; radius: Theme.radius
        color: btnEnabled ? (btnArea.containsMouse ? Theme.surfaceAlt : "transparent") : "transparent"
        opacity: btnEnabled ? 1.0 : 0.38

        Column {
            anchors.centerIn: parent
            spacing: 3
            Image {
                visible: btn.iconFile !== ""
                anchors.horizontalCenter: parent.horizontalCenter
                width: toolBar.small ? 20 : 24; height: toolBar.small ? 20 : 24
                sourceSize: Qt.size(48, 48)
                smooth: true
                source: btn.iconFile !== "" ? "qrc:/icons/" + btn.iconFile + "-48.png" : ""
            }
            Text {
                visible: btn.iconFile === ""
                anchors.horizontalCenter: parent.horizontalCenter
                text: btn.icon
                color: btnArea.containsMouse && btn.btnEnabled ? Theme.accent : Theme.textDim
                font.pixelSize: 16
            }
            Text {
                visible: !toolBar.small
                anchors.horizontalCenter: parent.horizontalCenter
                text: btn.label
                color: Theme.textDim
                font.pixelSize: 10
            }
        }
        MouseArea {
            id: btnArea
            anchors.fill: parent
            hoverEnabled: true
            enabled: btn.btnEnabled
            onClicked: btn.onAction(btn.actionId)
        }
    }

    Row {
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: parent.left
        anchors.leftMargin: 6
        spacing: 2

        // Downloads actions
        Row {
            visible: toolBar.workspaceIndex === 1
            spacing: 2
            ToolBtn { actionId: "dl.add"; iconName: "list-add"; label: qsTr("Add URL"); icon: "+"; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.resume"; iconName: "media-playback-start"; label: qsTr("Resume"); icon: "▶"; btnEnabled: toolBar.canResume; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.stop"; iconName: "media-playback-stop"; label: qsTr("Stop"); icon: "■"; btnEnabled: toolBar.selectedDownloads.length > 0; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.stopAll"; iconName: "media-playback-pause"; label: qsTr("Stop All"); icon: "■■"; onAction: toolBar.onAction }
            Rectangle { width: 1; height: 32; anchors.verticalCenter: parent.verticalCenter; color: Theme.border }
            ToolBtn { actionId: "dl.delete"; iconName: "edit-delete"; label: qsTr("Delete"); icon: "✕"; btnEnabled: toolBar.selectedDownloads.length > 0; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.clearCompleted"; iconName: "dialog-ok-apply"; label: qsTr("Del. Done"); icon: "✕✓"; onAction: toolBar.onAction }
            Rectangle { width: 1; height: 32; anchors.verticalCenter: parent.verticalCenter; color: Theme.border }
            ToolBtn { actionId: "dl.settings"; iconName: "configure"; label: qsTr("Options"); icon: "⚙︎"; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.scheduler"; label: qsTr("Scheduler"); icon: "◷"; onAction: toolBar.onAction }
            Rectangle { width: 1; height: 32; anchors.verticalCenter: parent.verticalCenter; color: Theme.border }
            ToolBtn { actionId: "dl.startQueue"; iconName: "media-playback-start"; label: qsTr("Start Queue"); icon: "▶"; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.stopQueue"; iconName: "media-playback-stop"; label: qsTr("Stop Queue"); icon: "■"; onAction: toolBar.onAction }
        }

        // Torrent actions
        Row {
            visible: toolBar.workspaceIndex === 2
            spacing: 2
            ToolBtn { actionId: "torrent.add"; iconName: "list-add"; label: qsTr("Add Torrent"); icon: "+"; onAction: toolBar.onAction }
            ToolBtn { actionId: "torrent.start"; iconName: "media-playback-start"; label: qsTr("Start"); icon: "▶"; btnEnabled: toolBar.torrentCanStart; onAction: toolBar.onAction }
            ToolBtn { actionId: "torrent.pause"; iconName: "media-playback-pause"; label: qsTr("Pause"); icon: "‖"; btnEnabled: toolBar.torrentCanPause; onAction: toolBar.onAction }
            ToolBtn { actionId: "torrent.resume"; iconName: "media-playback-start"; label: qsTr("Resume"); icon: "▶"; btnEnabled: toolBar.torrentCanResume; onAction: toolBar.onAction }
            Rectangle { width: 1; height: 32; anchors.verticalCenter: parent.verticalCenter; color: Theme.border }
            ToolBtn { actionId: "torrent.delete"; iconName: "edit-delete"; label: qsTr("Delete"); icon: "✕"; btnEnabled: toolBar.selectedTorrents.length > 0; onAction: toolBar.onAction }
        }

        // Overview actions
        Row {
            visible: toolBar.workspaceIndex === 0
            spacing: 2
            ToolBtn { actionId: "dl.add"; iconName: "list-add"; label: qsTr("Add URL"); icon: "+"; onAction: toolBar.onAction }
        }
    }

    // Selection info (right side)
    Row {
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: 10
        spacing: 8

        Text {
            visible: toolBar.workspaceIndex === 1 && toolBar.selectedDownloads.length > 0
            text: toolBar.selectedDownloads.length + qsTr(" selected")
            color: Theme.textDim; font.pixelSize: 11
        }
        Text {
            visible: toolBar.workspaceIndex === 2 && toolBar.selectedTorrents.length > 0
            text: toolBar.selectedTorrents.length + qsTr(" selected")
            color: Theme.textDim; font.pixelSize: 11
        }
    }
}
