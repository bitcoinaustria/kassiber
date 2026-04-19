import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"

ScrollView {
    id: root
    clip: true

    ColumnLayout {
        width: root.availableWidth
        spacing: theme.spacingLg

        Card {
            Layout.fillWidth: true
            fillColor: theme.cardAlt

            ColumnLayout {
                anchors.fill: parent
                spacing: theme.spacingMd

                Text {
                    text: "Settings"
                    color: theme.ink
                    font.family: theme.displayFont
                    font.pixelSize: 32
                    font.bold: true
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "This page now carries the settings mockup's read-only backbone: runtime paths, profile policy, and managed state locations."
                    color: theme.inkMuted
                    font.family: theme.bodyFont
                    font.pixelSize: 14
                }
            }
        }

        Repeater {
            model: settingsVM.cards

            Card {
                Layout.fillWidth: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: theme.spacingSm

                    Text {
                        text: modelData["label"]
                        color: theme.inkMuted
                        font.family: theme.bodyFont
                        font.pixelSize: 12
                        font.bold: true
                    }

                    Text {
                        Layout.fillWidth: true
                        wrapMode: Text.WrapAnywhere
                        text: modelData["value"]
                        color: theme.ink
                        font.family: theme.bodyFont
                        font.pixelSize: 13
                    }

                    Text {
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                        text: modelData["hint"]
                        color: theme.inkMuted
                        font.family: theme.bodyFont
                        font.pixelSize: 12
                    }
                }
            }
        }
    }
}
