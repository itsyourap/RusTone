/*
 * Small C helpers compiled with the same compiler and headers as
 * pjproject. Later phases add the custom video renderer here.
 */
#include <pjsua-lib/pjsua.h>

/* pjsip_cfg() is PJ_INLINE in static builds, so it has no exported symbol. */
pjsip_cfg_t *jf_pjsip_cfg(void) { return pjsip_cfg(); }

/* Struct sizes as MSVC sees them, used by tests to check the bindgen
 * layouts against the compiler that built the library.
 */
size_t jf_sizeof_pjsua_config(void)           { return sizeof(pjsua_config); }
size_t jf_sizeof_pjsua_media_config(void)     { return sizeof(pjsua_media_config); }
size_t jf_sizeof_pjsua_logging_config(void)   { return sizeof(pjsua_logging_config); }
size_t jf_sizeof_pjsua_transport_config(void) { return sizeof(pjsua_transport_config); }
size_t jf_sizeof_pjsua_acc_config(void)       { return sizeof(pjsua_acc_config); }
size_t jf_sizeof_pjsua_acc_info(void)         { return sizeof(pjsua_acc_info); }
size_t jf_sizeof_pjsua_call_info(void)        { return sizeof(pjsua_call_info); }
size_t jf_sizeof_pjsua_call_setting(void)     { return sizeof(pjsua_call_setting); }
size_t jf_sizeof_pjsua_codec_info(void)       { return sizeof(pjsua_codec_info); }
