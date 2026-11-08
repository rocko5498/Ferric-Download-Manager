import QtQuick

Rectangle {
    id: card
    property string heading: ""
    default property alias content: body.data
    width: parent ? parent.width : 0
    height: wrap.implicitHeight + 2 * Theme.sp3
    color: Theme.surface; radius: Theme.radiusCard; border.color: Theme.border; border.width: 1
    Column {
        id: wrap
        anchors { fill: parent; margins: Theme.sp3 }
        spacing: Theme.sp3
        Text { visible: card.heading !== ""; text: card.heading; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.weight: Font.DemiBold }
        Column { id: body; width: parent.width; spacing: Theme.sp3 }
    }
}
