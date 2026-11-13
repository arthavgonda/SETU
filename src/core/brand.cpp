#include "brand.h"
#include "settings.h"

Brand::Brand(QObject *parent)
    : QObject(parent)
{
    if (auto *s = SettingsStore::instance())
        connect(s, &SettingsStore::valuesChanged, this, &Brand::assistantChanged);
}

QString Brand::productName() const
{
    return QStringLiteral(SETU_PRODUCT_NAME);
}

QString Brand::fullName() const
{
    return QStringLiteral(SETU_PRODUCT_FULL_NAME);
}

QString Brand::version() const
{
    return QStringLiteral(SETU_VERSION);
}

QString Brand::build() const
{
    return QStringLiteral(SETU_BUILD_ID);
}

QString Brand::assistantName() const
{
    return productName();
}

QString Brand::wakePhrase() const
{
    return tr("Hey %1").arg(assistantName());
}

QString Brand::supportUrl() const
{
    return QStringLiteral(SETU_SUPPORT_URL);
}

QString Brand::feedbackAddress() const
{
    return QStringLiteral(SETU_FEEDBACK_ADDRESS);
}
