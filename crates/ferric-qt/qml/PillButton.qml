import QtQuick

Rectangle {
    id: pb
    property string label: ""
    property bool primary: true
    signal clicked()
    width: Math.max(Theme.btnM, pbText.implicitWidth + 2 * Theme.sp4); height: Theme.controlH; radius: Theme.radius
    color: primary ? (pbArea.containsMouse ? Theme.accentDark : Theme.accent) : (pbArea.containsMouse ? Theme.border : Theme.surfaceAlt)
    Text { id: pbText; anchors.centerIn: parent; text: pb.label; color: pb.primary ? "white" : Theme.text; font.pixelSize: Theme.fontSizeNormal; font.weight: Font.DemiBold }
    MouseArea { id: pbArea; anchors.fill: parent; hoverEnabled: true; onClicked: pb.clicked() }
}
