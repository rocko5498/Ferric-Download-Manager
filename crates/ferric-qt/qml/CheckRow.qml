import QtQuick

Row {
    id: cr
    property string label: ""
    property bool checked: false
    signal toggled()
    spacing: Theme.sp2
    Rectangle {
        anchors.verticalCenter: parent.verticalCenter
        width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
        color: cr.checked ? Theme.accent : Theme.surface; border.color: cr.checked ? Theme.accent : Theme.border; border.width: 1
        Text { anchors.centerIn: parent; text: cr.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
        MouseArea { anchors.fill: parent; onClicked: cr.toggled() }
    }
    Text { anchors.verticalCenter: parent.verticalCenter; text: cr.label; color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
}
