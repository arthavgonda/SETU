#include "i18n.h"
#include "settings.h"

#include <QCoreApplication>
#include <QFile>
#include <QGuiApplication>
#include <QLocale>

namespace {
Translator *g_translator = nullptr;

struct Lang {
    const char *code;
    const char *native;
    const char *english;
    bool rtl;
};

const Lang kLangs[] = {
    {"en", "English", "English", false},       {"hi", "हिन्दी", "Hindi", false},   {"es", "Español", "Spanish", false},
    {"fr", "Français", "French", false},       {"de", "Deutsch", "German", false}, {"pt", "Português", "Portuguese", false},
    {"ar", "العربية", "Arabic", true},          {"ja", "日本語", "Japanese", false}, {"zh", "中文", "Chinese", false},
    {"qps-ploc", "Ƥşéüðø", "Pseudo", false},  {"qps-plocm", "Ƥşéüðø ᴙ", "Pseudo mirrored", true},
};

bool available(const QString &code)
{
    return code == QLatin1String("en") || QFile::exists(QStringLiteral(":/i18n/setu_%1.qm").arg(code));
}
}

Translator::Translator(QObject *parent)
    : QObject(parent)
{
    g_translator = this;
    const QString forced = qEnvironmentVariable("SETU_UI_LANGUAGE");
    apply(forced.isEmpty() ? QStringLiteral("en") : forced);
    if (auto *s = SettingsStore::instance()) {
        connect(s, &SettingsStore::valuesChanged, this, [this, s] {
            const QString forced = qEnvironmentVariable("SETU_UI_LANGUAGE");
            const QString l = forced.isEmpty() ? s->get(QStringLiteral("ui.language"), QStringLiteral("en")).toString() : forced;
            if (l != m_language)
                apply(l);
        });
    }
}

Translator *Translator::instance()
{
    return g_translator;
}

Translator *Translator::create(QQmlEngine *, QJSEngine *)
{
    QJSEngine::setObjectOwnership(g_translator, QJSEngine::CppOwnership);
    return g_translator;
}

QVariantList Translator::languages() const
{
    QVariantList out;
    for (const auto &l : kLangs) {
        const QString code = QString::fromLatin1(l.code);
        if (!available(code))
            continue;
        out.append(QVariantMap{{QStringLiteral("code"), code}, {QStringLiteral("native"), QString::fromUtf8(l.native)},
                               {QStringLiteral("english"), QString::fromLatin1(l.english)}, {QStringLiteral("rtl"), l.rtl}});
    }
    return out;
}

void Translator::apply(const QString &language)
{
    const QString lang = available(language) ? language : QStringLiteral("en");
    if (m_translator) {
        QCoreApplication::removeTranslator(m_translator.get());
        m_translator.reset();
    }
    if (QFile::exists(QStringLiteral(":/i18n/setu_%1.qm").arg(lang))) {
        auto t = std::make_unique<QTranslator>();
        if (t->load(QStringLiteral(":/i18n/setu_%1.qm").arg(lang))) {
            QCoreApplication::installTranslator(t.get());
            m_translator = std::move(t);
        }
    }
    bool rtl = false;
    for (const auto &l : kLangs)
        if (lang == QLatin1String(l.code))
            rtl = l.rtl;
    QGuiApplication::setLayoutDirection(rtl ? Qt::RightToLeft : Qt::LeftToRight);
    QLocale::setDefault(QLocale(lang.startsWith(QLatin1String("qps")) ? QStringLiteral("en") : lang));
    m_language = lang;
    if (m_engine)
        m_engine->retranslate();
    emit languageChanged();
}
