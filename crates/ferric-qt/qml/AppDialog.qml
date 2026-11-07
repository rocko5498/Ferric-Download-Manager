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
    property int cardMaxHeight: 610
    property int cardWidth: Theme.dialogW
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

    // Soft shadow: two translucent layers under the card
    Rectangle { x: card.x - Theme.sp1; y: card.y + Theme.sp2; width: card.width + 2 * Theme.sp1; height: card.height + Theme.sp1; radius: card.radius + Theme.sp1; color: "#14000000" }
    Rectangle { x: card.x; y: card.y + Theme.sp1; width: card.width; height: card.height; radius: card.radius; color: Theme.shadow }

    // Centered dialog card — fixed width, no height binding cycle
    Rectangle {
        id: card
        width: Math.min(parent.width - 2 * Theme.sp5, dialog.cardWidth)
        // as tall as its content needs (never shorter than 233, never taller than the window or cardMaxHeight)
        height: Math.max(233, Math.min(parent.height - 2 * Theme.sp5, dialog.cardMaxHeight,
            titleRow.height + Math.max(contentColumn.implicitHeight, contentColumn.childrenRect.height) + buttonRow.height + 3 * Theme.sp4 + Theme.sp3))
        anchors.centerIn: parent
        radius: Theme.radiusCard
        color: Theme.surface

        // Block clicks from propagating to scrim
        MouseArea { anchors.fill: parent }

        // Title row
        Row {
            id: titleRow
            anchors { top: parent.top; left: parent.left; right: parent.right; topMargin: Theme.sp4; leftMargin: Theme.sp4; rightMargin: Theme.sp4 }
            height: dialog.title.length > 0 ? Theme.sp5 : 0
            visible: dialog.title.length > 0
            Text {
                text: dialog.title
                color: Theme.text; font.pixelSize: Theme.fontSizeMedium; font.bold: true
            }
        }

        // Content area — Item wrapper breaks Column implicitHeight ↔ anchor cycle (R1 fix)
        Item {
            id: contentArea
            anchors { top: titleRow.bottom; left: parent.left; right: parent.right; bottom: buttonRow.top; topMargin: dialog.title.length > 0 ? Theme.sp2 : Theme.sp4; leftMargin: Theme.sp4; rightMargin: Theme.sp4; bottomMargin: Theme.sp4 }

            Column {
                id: contentColumn
                anchors.fill: parent
                spacing: Theme.sp2
                clip: true
            }
        }

        // Button bar
        Row {
            id: buttonRow
            anchors { bottom: parent.bottom; horizontalCenter: parent.horizontalCenter; bottomMargin: Theme.sp4 }
            spacing: Theme.sp2
        }
    }
}
