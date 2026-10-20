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

/*
 * Prints header `name` of a received message (any header type) into `buf`
 * as "Name: value", NUL-terminated and truncated to fit. Returns the
 * printed length, or -1 when the message has no such header. Used for
 * caller ID (P-Asserted-Identity, Remote-Party-ID), Diversion, User-Agent.
 */
int jf_rdata_header(const pjsip_rx_data *rdata, const char *name,
                    char *buf, size_t buf_len)
{
    pj_str_t hname;
    const pjsip_hdr *hdr;
    int len;

    if (!rdata || !rdata->msg_info.msg || !name || !buf || buf_len < 2)
        return -1;
    hname = pj_str((char *)name);
    hdr = (const pjsip_hdr *)pjsip_msg_find_hdr_by_name(rdata->msg_info.msg,
                                                        &hname, NULL);
    if (!hdr)
        return -1;
    len = pjsip_hdr_print_on((void *)hdr, buf, buf_len - 1);
    if (len < 0)
        len = 0;
    buf[len] = '\0';
    return len;
}

/* Struct sizes as MSVC sees them, used by tests to check the bindgen
 * layouts against the compiler that built the library.
 */
/* Force codec: answer an incoming offer with our codec order
 * instead of the caller's. Call from on_incoming_call. */
pj_status_t jf_prefer_local_codec_order(pjsip_rx_data *rdata)
{
    pjsip_dialog *dlg = pjsip_rdata_get_dlg(rdata);
    pjsip_inv_session *inv = dlg ? pjsip_dlg_get_inv_session(dlg) : NULL;
    if (!inv || !inv->neg)
        return PJ_EINVALIDOP;
    return pjmedia_sdp_neg_set_prefer_remote_codec_order(inv->neg, PJ_FALSE);
}

/*
 * Copies a received message's body into `buf` and its media type
 * ("application/simple-message-summary") into `ctype`, both NUL-terminated
 * and truncated to fit. Returns the body's full length, or -1 without one.
 */
int jf_rdata_body(const pjsip_rx_data *rdata, char *buf, size_t buf_len,
                  char *ctype, size_t ctype_len)
{
    const pjsip_msg_body *body;
    size_t n;

    if (!rdata || !rdata->msg_info.msg || !buf || buf_len < 1)
        return -1;
    body = rdata->msg_info.msg->body;
    if (!body || !body->len)
        return -1;
    n = body->len < buf_len - 1 ? body->len : buf_len - 1;
    pj_memcpy(buf, body->data, n);
    buf[n] = '\0';
    if (ctype && ctype_len) {
        int len = pj_ansi_snprintf(ctype, ctype_len, "%.*s/%.*s",
                                   (int)body->content_type.type.slen,
                                   body->content_type.type.ptr,
                                   (int)body->content_type.subtype.slen,
                                   body->content_type.subtype.ptr);
        if (len < 0)
            ctype[0] = '\0';
    }
    return (int)body->len;
}

/*
 * Adds the header "name: value" to a message's extra headers. `storage`
 * is caller memory of jf_sizeof_generic_string_hdr() bytes that, like the
 * strings and `msg`, must stay put until the request or response is sent.
 */
void jf_msg_data_add_hdr(pjsua_msg_data *msg, void *storage,
                         const char *name, const char *value)
{
    pjsip_generic_string_hdr *hdr = (pjsip_generic_string_hdr *)storage;
    pj_str_t hname = pj_str((char *)name);
    pj_str_t hvalue = pj_str((char *)value);

    pjsip_generic_string_hdr_init2(hdr, &hname, &hvalue);
    pj_list_push_back(&msg->hdr_list, hdr);
}

size_t jf_sizeof_generic_string_hdr(void)
{
    return sizeof(pjsip_generic_string_hdr);
}

size_t jf_sizeof_pjsua_config(void)           { return sizeof(pjsua_config); }
size_t jf_sizeof_pjsua_media_config(void)     { return sizeof(pjsua_media_config); }
size_t jf_sizeof_pjsua_logging_config(void)   { return sizeof(pjsua_logging_config); }
size_t jf_sizeof_pjsua_transport_config(void) { return sizeof(pjsua_transport_config); }
size_t jf_sizeof_pjsua_acc_config(void)       { return sizeof(pjsua_acc_config); }
size_t jf_sizeof_pjsua_acc_info(void)         { return sizeof(pjsua_acc_info); }
size_t jf_sizeof_pjsua_call_info(void)        { return sizeof(pjsua_call_info); }
size_t jf_sizeof_pjsua_call_setting(void)     { return sizeof(pjsua_call_setting); }
size_t jf_sizeof_pjsua_codec_info(void)       { return sizeof(pjsua_codec_info); }
