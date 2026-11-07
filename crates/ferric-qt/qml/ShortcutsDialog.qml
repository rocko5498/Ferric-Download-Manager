import QtQuick

// FQT-089: Keyboard shortcuts dialog \u2014 extracted from main.qml
AppDialog {
    id: shortcutsDialog

    property var shortcutsModel: [
        ["Ctrl+N", "Add URL"],
        ["Ctrl+Shift+N", "Add batch URLs"],
        ["Ctrl+O", "Add torrent/magnet"],
        ["Ctrl+T", "Add torrent/magnet"],
        ["Delete", "Delete selected"],
        ["Ctrl+A", "Select all"],
        ["Ctrl+P", "Preferences"],
        ["Ctrl+Q", "Quit"],
        ["F5", "Refresh"],
        ["Space", "Pause/Resume selected"],
        ["Ctrl+L", "Clear completed"],
        ["Escape", "Close dialog / Deselect"]
    ]

    title: qsTr("Keyboard Shortcuts")

    contentItem: [
        Flickable {
            width: parent.width; height: 380; clip: true
            contentHeight: shortcutsCol.height
            Column {
                id: shortcutsCol
                width: parent.width
                spacing: Theme.sp1

                Repeater {
                    model: shortcutsDialog.shortcutsModel
                    Row {
                        spacing: Theme.sp3; height: 22; width: shortcutsCol.width
                        Rectangle {
                            width: 100; height: 20; radius: 3
                            color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                            Text {
                                anchors.centerIn: parent
                                text: modelData[0]
                                color: Theme.accent; font.pixelSize: Theme.fontSizeSmall; font.family: "monospace"
                            }
                        }
                        Text { text: modelData[1]; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                    }
                }
            }
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius
            color: scCloseArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            MouseArea {
                id: scCloseArea
                anchors.fill: parent
                hoverEnabled: true
                onClicked: shortcutsDialog.close()
            }
        }
    ]
}
