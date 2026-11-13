#pragma once

#include <QString>

class QQmlEngine;
class QGuiApplication;

namespace setu {
void loadFonts(bool display);
void prepare(QGuiApplication &app, const QString &appName);
void install(QQmlEngine &engine);
}
