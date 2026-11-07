import QtQuick

// Base modal dialog: scrim + focus trap + Esc close + title + button bar.
Rectangle {
    id: dialog

    visible: false
    z: 100
    x: 0; y: 0
    width: rootWidth; height: rootHeight
    color: "#CC000000"
    focus: true

    // Default to parent size so dialogs without explicit bindings still scrim the full window
    property int rootWidth: parent ? parent.width : 800
    property int rootHeight: parent ? parent.height : 600
    property bool appModal: true
    property int cardMaxHeight: 400
    property string title: ""
    property alias contentItem: contentColumn.data
    property alias buttons: buttonRow.data

    signal closed()

    function open() { visible = true; forceActiveFocus() }
    function close() { visible = false; closed() }

    onVisibleChanged: if (visible) Qt.callLater(function() {
        if (dialog.visible)
            dialog.forceActiveFocus()
    })
    Keys.onEscapePressed: dialog.visible = false

    // Focus trap: click on scrim closes dialog
    MouseArea {
        anchors.fill: parent
        onClicked: dialog.visible = false
    }

    // Centered dialog card — fixed width, no height binding cycle
    Rectangle {
        id: card
        width: Math.min(parent.width - 80, 520)
        height: Math.min(parent.height - 80, cardMaxHeight)
        anchors.centerIn: parent
        radius: 6
        color: Theme.surface

        // Block clicks from propagating to scrim
        MouseArea { anchors.fill: parent }

        // Title row
        Row {
            id: titleRow
            anchors { top: parent.top; left: parent.left; right: parent.right; topMargin: 16; leftMargin: 20; rightMargin: 20 }
            height: dialog.title.length > 0 ? 24 : 0
            visible: dialog.title.length > 0
            Text {
                text: dialog.title
                color: Theme.text; font.pixelSize: 15; font.bold: true
            }
        }

        // Content area — Item wrapper breaks Column implicitHeight ↔ anchor cycle (R1 fix)
        Item {
            id: contentArea
            anchors { top: titleRow.bottom; left: parent.left; right: parent.right; bottom: buttonRow.top; topMargin: dialog.title.length > 0 ? 12 : 16; leftMargin: 20; rightMargin: 20; bottomMargin: 16 }

            Column {
                id: contentColumn
                anchors.fill: parent
                spacing: 8
                clip: true
            }
        }

        // Button bar
        Row {
            id: buttonRow
            anchors { bottom: parent.bottom; horizontalCenter: parent.horizontalCenter; bottomMargin: 16 }
            spacing: 8
        }
    }
}
