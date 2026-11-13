#include "setucore.h"
#include "daemonclient.h"
#include "i18n.h"
#include "iconprovider.h"
#include "settings.h"

#include <QFont>
#include <QFontDatabase>
#include <QGuiApplication>
#include <QQmlEngine>
#include <QQuickWindow>

namespace setu {

void loadFonts(bool display)
{
    for (const char *f : {"IBMPlexSans-Regular.ttf", "IBMPlexSans-Medium.ttf", "IBMPlexSans-SemiBold.ttf", "IBMPlexMono-Regular.ttf", "IBMPlexMono-Medium.ttf",
                          "AtkinsonHyperlegible-Regular.ttf", "AtkinsonHyperlegible-Bold.ttf", "NotoSansDevanagari-Variable.ttf"})
        QFontDatabase::addApplicationFont(QStringLiteral(":/fonts/%1").arg(QLatin1String(f)));
    if (display)
        QFontDatabase::addApplicationFont(QStringLiteral(":/fonts/SourceSerif4-Variable.ttf"));
    QFont::insertSubstitutions(QStringLiteral("IBM Plex Sans"), {QStringLiteral("Noto Sans Devanagari"), QStringLiteral("Noto Sans Arabic"), QStringLiteral("Noto Sans CJK SC")});
    QFont::insertSubstitutions(QStringLiteral("Atkinson Hyperlegible"), {QStringLiteral("Noto Sans Devanagari"), QStringLiteral("Noto Sans Arabic")});
    QFont base(QStringLiteral("IBM Plex Sans"));
    base.setPointSizeF(QGuiApplication::font().pointSizeF());
    QGuiApplication::setFont(base);
}

void prepare(QGuiApplication &app, const QString &appName)
{
    app.setApplicationName(appName);
    app.setApplicationVersion(QStringLiteral(SETU_VERSION));
    app.setOrganizationName(QStringLiteral(SETU_PRODUCT_NAME));
    app.setDesktopFileName(QStringLiteral("ai.setu.Setu"));
    loadFonts(appName != QLatin1String("setu-shell"));
}

void install(QQmlEngine &engine)
{
    engine.addImageProvider(QStringLiteral("icon"), new IconProvider);
    engine.addImageProvider(QStringLiteral("tray"), new TrayIconProvider);
    engine.addImageProvider(QStringLiteral("appicon"), new AppIconProvider);
    if (auto *t = Translator::instance())
        t->setEngine(&engine);
}

}
