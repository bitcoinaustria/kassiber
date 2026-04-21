import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

Control {
    id: root
    property string label: ""
    property bool mono: false
    property string rightText: ""
    property bool invalid: false
    property bool dense: false
    property alias text: field.text
    property alias placeholderText: field.placeholderText
    property alias echoMode: field.echoMode
    property alias readOnly: field.readOnly
    property alias validator: field.validator
    property alias inputMethodHints: field.inputMethodHints
    property alias maximumLength: field.maximumLength
    property alias selectByMouse: field.selectByMouse
    property alias inputMask: field.inputMask
    property alias horizontalAlignment: field.horizontalAlignment
    property alias cursorPosition: field.cursorPosition
    property alias fieldItem: field
    readonly property bool acceptableInput: field.acceptableInput

    signal editingFinished()
    signal accepted()

    implicitWidth: Math.max(220, column.implicitWidth)
    implicitHeight: column.implicitHeight

    contentItem: Column {
        id: column
        spacing: 6

        Text {
            visible: root.label.length > 0
            text: root.label
            color: Design.ink2(theme)
            font.family: Design.sans()
            font.pixelSize: 10
            font.weight: Font.DemiBold
            font.capitalization: Font.AllUppercase
            font.letterSpacing: 1.4
        }

        Rectangle {
            id: shell
            implicitWidth: Math.max(200, field.implicitWidth + adornment.implicitWidth + 20)
            implicitHeight: root.dense ? 32 : 36
            color: Design.paperAlt(theme)
            border.color: root.invalid ? Design.err(theme) : (field.activeFocus ? Design.ink(theme) : Design.line(theme))
            border.width: 1
            radius: 2

            Row {
                anchors.fill: parent
                anchors.leftMargin: 10
                anchors.rightMargin: 10
                spacing: 8

                TextField {
                    id: field
                    width: parent.width - (adornment.visible ? adornment.implicitWidth : 0) - (adornment.visible ? parent.spacing : 0)
                    height: parent.height
                    color: Design.ink(theme)
                    font.family: root.mono ? Design.mono(theme) : Design.sans()
                    font.pixelSize: root.mono ? 12 : 13
                    placeholderTextColor: Design.ink3(theme)
                    selectionColor: Design.accent(theme)
                    selectedTextColor: Design.paper(theme)
                    verticalAlignment: TextInput.AlignVCenter
                    background: Rectangle {
                        color: "transparent"
                        border.width: 0
                    }
                    padding: 0

                    onEditingFinished: root.editingFinished()
                    onAccepted: root.accepted()
                }

                Text {
                    id: adornment
                    visible: root.rightText.length > 0
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.rightText
                    color: Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: 11
                }
            }
        }
    }
}
