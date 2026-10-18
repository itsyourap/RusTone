/*
 * Small C helpers compiled with the same compiler and headers as
 * pjproject. Later phases add the custom video renderer here.
 */
#include <pjsua-lib/pjsua.h>

/* pjsip_cfg() is PJ_INLINE in static builds, so it has no exported symbol. */
pjsip_cfg_t *jf_pjsip_cfg(void) { return pjsip_cfg(); }

/*
 * Copies the value of header `name` from the response that completed a
 * registration (e.g. "P-Registrar-Error") into `buf`.
 * Returns the value's full length, or -1 when there is no such header.
 */
int jf_reg_info_header(const pjsua_reg_info *info, const char *name,
                       char *buf, size_t buf_len)
{
    pj_str_t hname;
    const pjsip_generic_string_hdr *hdr;
    size_t n;

    if (!info || !info->cbparam || !info->cbparam->rdata || !name)
        return -1;
    hname = pj_str((char *)name);
    hdr = (const pjsip_generic_string_hdr *)pjsip_msg_find_hdr_by_name(
              info->cbparam->rdata->msg_info.msg, &hname, NULL);
    if (!hdr)
        return -1;
    if (buf && buf_len) {
        n = (size_t)hdr->hvalue.slen < buf_len - 1 ? (size_t)hdr->hvalue.slen
                                                   : buf_len - 1;
        pj_memcpy(buf, hdr->hvalue.ptr, n);
        buf[n] = '\0';
    }
    return (int)hdr->hvalue.slen;
}

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
