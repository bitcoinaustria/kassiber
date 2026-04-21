import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "Design.js" as Design

Item {
    id: root
    property string label: ""
    property string subtitle: ""
    property string totalLabel: ""
    property bool negative: false
    property bool hideSensitive: false
    property bool bottomBorder: true
    property var children_: []
    // When true, a clickable chevron is shown and children start collapsed.
    // Rows without children render without a chevron and ignore expansion.
    property bool expanded: false

    signal toggled()

    readonly property bool hasChildren: children_ && children_.length > 0

    implicitWidth: 360
    implicitHeight: column.implicitHeight

    Column {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        spacing: 0

        // Parent row (header) --------------------------------------------
        Item {
            id: parentRow
            width: parent.width
            height: theme.rowHeightDefault + 8

            Rectangle {
                visible: root.bottomBorder
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: Design.line(theme)
            }

            Button {
                anchors.fill: parent
                flat: true
                padding: 0
                hoverEnabled: true
                enabled: root.hasChildren
                onClicked: if (root.hasChildren) root.toggled()

                contentItem: RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 2
                    anchors.rightMargin: 2
                    spacing: theme.spacingSm + 2

                    // Chevron (hidden when no children)
                    Text {
                        visible: root.hasChildren
                        Layout.preferredWidth: visible ? 12 : 0
                        horizontalAlignment: Text.AlignHCenter
                        text: root.expanded ? "\u25be" : "\u25b8"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontCaption
                    }

                    Text {
                        text: root.label
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontHeadingXs
                    }

                    Text {
                        text: root.subtitle
                        color: Design.ink3(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontBodySmall
                    }

                    Item { Layout.fillWidth: true }

                    Text {
                        text: root.hideSensitive
                            ? "\u2022 \u2022 \u2022 \u2022"
                            : "\u20bf " + root.totalLabel + " sat"
                        color: root.negative ? Design.accent(theme) : Design.ink(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBody
                        font.letterSpacing: -0.1
                    }
                }

                background: Rectangle { color: "transparent" }
            }
        }

        // Child rows (only rendered when expanded) -----------------------
        Repeater {
            model: root.expanded && root.hasChildren ? root.children_ : []

            delegate: Item {
                width: root.width
                height: theme.rowHeightCompact - 4

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: Design.line(theme)
                    opacity: 0.6
                }

                Text {
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: theme.spacingSm + 10
                    text: "\u21b3 " + modelData.label
                    color: Design.ink2(theme)
                    font.family: Design.sans()
                    font.pixelSize: theme.fontBodySmall
                }

                Text {
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.rightMargin: 2
                    text: root.hideSensitive
                        ? "\u2022 \u2022 \u2022 \u2022"
                        : "\u20bf " + modelData.value
                    color: Design.ink2(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontBodySmall
                }
            }
        }
    }
}
