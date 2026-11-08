import QtQuick

Column {
    id: lf
    property string label: ""
    property string placeholder: ""
    property int fieldWidth: 233
    property int hints: Qt.ImhNone
    property alias text: input.text
    signal accepted()
    spacing: Theme.sp1
    Text { text: lf.label; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; font.weight: Font.DemiBold }
    Rectangle {
        width: lf.fieldWidth; height: Theme.controlH; radius: Theme.radius
        color: Theme.bg; border.width: 1; border.color: input.activeFocus ? Theme.accent : Theme.border
        TextInput {
            id: input
            clip: true
            anchors { fill: parent; leftMargin: Theme.sp2; rightMargin: Theme.sp2 }
            verticalAlignment: TextInput.AlignVCenter
            color: Theme.text; font.pixelSize: Theme.fontSizeNormal
            inputMethodHints: lf.hints
            onAccepted: lf.accepted()
            Text { visible: input.text.length === 0; anchors.verticalCenter: parent.verticalCenter; text: lf.placeholder; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
        }
    }
}
