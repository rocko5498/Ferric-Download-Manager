import QtQuick

Rectangle {
    id: toolBar

    property bool small: false
    height: workspaceIndex > 1 ? 0 : (small ? 38 : Theme.toolbarH)   // nothing to show on Grabber, Media and RSS
    visible: height > 0
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
        property bool primary: false   // the main action: tinted, bold orange label
        property var onAction: function(id) {}
        // Oxygen icon name (LGPL, see icons/LICENSE-NOTE.md); the glyph stays as fallback.
        property string iconName: ""
        readonly property string iconFile: iconName

        width: toolBar.small ? 34 : Theme.toolBtnW; height: toolBar.small ? 32 : Theme.toolBtnH; radius: Theme.radius
        color: !btnEnabled ? "transparent" : primary ? (btnArea.containsMouse ? Theme.border : Theme.accentSoft) : (btnArea.containsMouse ? Theme.surfaceAlt : "transparent")
        opacity: btnEnabled ? 1.0 : 0.38

        Column {
            anchors.centerIn: parent
            spacing: Theme.sp1
            Image {
                visible: btn.iconFile !== ""
                anchors.horizontalCenter: parent.horizontalCenter
                width: toolBar.small ? 20 : Theme.toolIcon; height: toolBar.small ? 20 : Theme.toolIcon
                sourceSize: Qt.size(96, 96)
                smooth: true
                source: btn.iconFile !== "" ? "qrc:/icons/" + btn.iconFile + "-48.png" : ""
            }
            Text {
                visible: btn.iconFile === ""
                anchors.horizontalCenter: parent.horizontalCenter
                text: btn.icon
                color: btnArea.containsMouse && btn.btnEnabled ? Theme.accent : Theme.textDim
                font.pixelSize: Theme.fontSizeMedium
            }
            Text {
                visible: !toolBar.small
                anchors.horizontalCenter: parent.horizontalCenter
                text: btn.label
                color: btn.primary ? Theme.accentDark : Theme.text
                font.weight: btn.primary ? Font.DemiBold : Font.Normal
                font.pixelSize: Theme.fontSizeNormal
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
        anchors.leftMargin: Theme.sp1
        spacing: 2

        // Downloads actions
        Row {
            visible: toolBar.workspaceIndex === 1
            spacing: 2
            ToolBtn { actionId: "dl.add"; primary: true; iconName: "tb-add"; label: qsTr("Add URL"); icon: "+"; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.resume"; iconName: "tb-resume"; label: qsTr("Resume"); icon: "▶"; btnEnabled: toolBar.canResume; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.stop"; iconName: "tb-stop"; label: qsTr("Stop"); icon: "■"; btnEnabled: toolBar.selectedDownloads.length > 0; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.stopAll"; iconName: "tb-stopall"; label: qsTr("Stop All"); icon: "■■"; onAction: toolBar.onAction }
            Rectangle { width: 1; height: 32; anchors.verticalCenter: parent.verticalCenter; color: Theme.border }
            ToolBtn { actionId: "dl.delete"; iconName: "tb-delete"; label: qsTr("Delete"); icon: "✕"; btnEnabled: toolBar.selectedDownloads.length > 0; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.clearCompleted"; iconName: "tb-deldone"; label: qsTr("Del. Done"); icon: "✕✓"; onAction: toolBar.onAction }
            Rectangle { width: 1; height: 32; anchors.verticalCenter: parent.verticalCenter; color: Theme.border }
            ToolBtn { actionId: "dl.settings"; iconName: "tb-options"; label: qsTr("Options"); icon: "⚙︎"; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.scheduler"; iconName: "tb-scheduler"; label: qsTr("Scheduler"); icon: "◷"; onAction: toolBar.onAction }
            Rectangle { width: 1; height: 32; anchors.verticalCenter: parent.verticalCenter; color: Theme.border }
            ToolBtn { actionId: "dl.startQueue"; iconName: "tb-startq"; label: qsTr("Start Queue"); icon: "▶"; onAction: toolBar.onAction }
            ToolBtn { actionId: "dl.stopQueue"; iconName: "tb-stopq"; label: qsTr("Stop Queue"); icon: "■"; onAction: toolBar.onAction }
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
            ToolBtn { actionId: "dl.add"; primary: true; iconName: "tb-add"; label: qsTr("Add URL"); icon: "+"; onAction: toolBar.onAction }
        }
    }

    // Selection info (right side)
    Row {
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: Theme.sp2
        spacing: Theme.sp2

        Text {
            anchors.verticalCenter: parent.verticalCenter
            visible: toolBar.workspaceIndex === 1 && toolBar.selectedDownloads.length > 0
            text: toolBar.selectedDownloads.length + qsTr(" selected")
            color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
        }
        Text {
            anchors.verticalCenter: parent.verticalCenter
            visible: toolBar.workspaceIndex === 2 && toolBar.selectedTorrents.length > 0
            text: toolBar.selectedTorrents.length + qsTr(" selected")
            color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
        }
    }
}
