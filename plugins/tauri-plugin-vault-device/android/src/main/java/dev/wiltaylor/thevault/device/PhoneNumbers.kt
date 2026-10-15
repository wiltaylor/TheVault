package dev.wiltaylor.thevault.device

import android.content.Context
import android.telephony.TelephonyManager
import com.google.i18n.phonenumbers.NumberParseException
import com.google.i18n.phonenumbers.PhoneNumberUtil
import java.util.Locale

/**
 * Normalises phone numbers to E.164 with the phone's region, as every device event payload
 * carries them (D24). A value that is not a valid phone number, such as a short code or a sender
 * name, normalises to null.
 */
class PhoneNumbers(region: String) {
    /** The ISO 3166 region numbers without a country code are read in, such as `AU`. */
    val region: String = region.uppercase(Locale.ROOT)

    private val util = PhoneNumberUtil.getInstance()

    /** [raw] in E.164, such as `+61412345678`, or null when it is not a phone number. */
    fun normalise(raw: String?): String? {
        if (raw.isNullOrBlank()) return null
        return try {
            val number = util.parse(raw, region)
            if (util.isValidNumber(number)) {
                util.format(number, PhoneNumberUtil.PhoneNumberFormat.E164)
            } else {
                null
            }
        } catch (_: NumberParseException) {
            null
        }
    }

    companion object {
        /** The phone's region: the network's country, else the SIM's, else the locale's. */
        fun forDevice(context: Context): PhoneNumbers {
            val telephony = context.getSystemService(TelephonyManager::class.java)
            val region = listOfNotNull(
                telephony?.networkCountryIso,
                telephony?.simCountryIso,
                Locale.getDefault().country,
            ).firstOrNull { it.length == 2 } ?: "US"
            return PhoneNumbers(region)
        }
    }
}
