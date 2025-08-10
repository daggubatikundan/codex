use url::Url;

pub(crate) fn build_success_url(
    url_base: &str,
    id_token: Option<&str>,
    org_id: Option<&str>,
    project_id: Option<&str>,
    plan_type: Option<&str>,
    needs_setup: bool,
    platform_url: &str,
) -> Result<Url, url::ParseError> {
    let mut success_url = Url::parse(&format!("{url_base}/success"))?;
    if let Some(id) = id_token {
        success_url.query_pairs_mut().append_pair("id_token", id);
    }
    if let Some(org) = org_id {
        success_url.query_pairs_mut().append_pair("org_id", org);
    }
    if let Some(proj) = project_id {
        success_url
            .query_pairs_mut()
            .append_pair("project_id", proj);
    }
    if let Some(pt) = plan_type {
        success_url.query_pairs_mut().append_pair("plan_type", pt);
    }
    success_url
        .query_pairs_mut()
        .append_pair("needs_setup", if needs_setup { "true" } else { "false" })
        .append_pair("platform_url", platform_url);
    Ok(success_url)
}
