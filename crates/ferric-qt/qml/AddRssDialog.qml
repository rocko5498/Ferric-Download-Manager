import QtQuick

// Add RSS Feed dialog \u2014 extracted from main.qml (Phase 2 split #10)
AppDialog {
    id: addRssDialog

    property var onAdd: function(url) {}

    title: qsTr("Add RSS Feed")

    contentItem: [
        Rectangle {
            width: parent.width; height: 28; color: Theme.bg
            border.color: rssUrlInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
            TextInput {
                clip: true
                id: rssUrlInput
                anchors { fill: parent; margins: Theme.sp1 }
                color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                Text { visible: parent.text.length === 0; text: qsTr("https://example.com/feed.xml"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; anchors.verticalCenter: parent.verticalCenter }
                Keys.onReturnPressed: addRssDialog.onAdd(text)
                Keys.onEscapePressed: addRssDialog.close()
            }
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: addRssDialog.close() }
        },
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Add"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: addRssDialog.onAdd(rssUrlInput.text) }
        }
    ]
}
