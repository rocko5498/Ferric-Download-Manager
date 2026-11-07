import QtQuick

// Labeled input field: label above, placeholder, clip, error state.
// Usage: FormField { label: "URL"; placeholder: "https://..."; text: urlInput.text; onTextChanged: ... }
Column {
    id: field

    property string label: ""
    property string placeholder: ""
    property alias text: input.text
    property string error: ""
    property int fieldWidth: 200
    property bool readOnly: false

    signal accepted()

    spacing: 4
    width: fieldWidth

    Text {
        visible: field.label.length > 0
        text: field.label
        color: Theme.textDim; font.pixelSize: 11
    }

    Rectangle {
        width: field.fieldWidth; height: 28; radius: Theme.radius
        color: Theme.bg
        border.color: field.error.length > 0 ? Theme.danger : (input.activeFocus ? Theme.accent : Theme.border)
        border.width: 1

        TextInput {
            id: input
            anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; leftMargin: 8; rightMargin: 8 }
            color: Theme.text; font.pixelSize: 12
            clip: true
            readOnly: field.readOnly
            selectByMouse: true
            Keys.onReturnPressed: field.accepted()

            Text {
                visible: !input.text && input.focus
                text: field.placeholder
                color: Theme.textDim; font.pixelSize: 12
                anchors.verticalCenter: parent.verticalCenter
            }
        }
    }

    Text {
        visible: field.error.length > 0
        text: field.error
        color: Theme.danger; font.pixelSize: 10
    }
}
