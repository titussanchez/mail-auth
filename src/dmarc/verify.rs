/*
 * Copyright (c) 2020-2022, Stalwart Labs Ltd.
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
 * option. This file may not be copied, modified, or distributed
 * except according to those terms.
 */

use std::sync::Arc;

use crate::{
    AuthenticatedMessage, DkimOutput, DkimResult, DmarcOutput, DmarcResult, Error, Resolver,
    SpfOutput, SpfResult,
};

use super::{Alignment, Dmarc, URI};

impl Resolver {
    /// Verifies the DMARC policy of an RFC5322.From domain
    pub async fn verify_dmarc(
        &self,
        message: &AuthenticatedMessage<'_>,
        dkim_output: &[DkimOutput<'_>],
        mail_from_domain: &str,
        spf_output: &SpfOutput,
    ) -> DmarcOutput {
        // Extract RFC5322.From
        let mut from_domain = "";
        for from in &message.from {
            if let Some((_, domain)) = from.rsplit_once('@') {
                if from_domain.is_empty() {
                    from_domain = domain;
                } else if from_domain != domain {
                    // Multi-valued RFC5322.From header fields with multiple
                    // domains MUST be exempt from DMARC checking.
                    return DmarcOutput::default();
                }
            }
        }
        if from_domain.is_empty() {
            return DmarcOutput::default();
        }

        // Obtain DMARC policy
        let dmarc = match self.dmarc_tree_walk(from_domain).await {
            Ok(Some(dmarc)) => dmarc,
            Ok(None) => return DmarcOutput::default().with_domain(from_domain),
            Err(err) => {
                let err = DmarcResult::from(err);
                return DmarcOutput::default()
                    .with_domain(from_domain)
                    .with_dkim_result(err.clone())
                    .with_spf_result(err);
            }
        };

        let mut output = DmarcOutput {
            spf_result: DmarcResult::None,
            dkim_result: DmarcResult::None,
            domain: from_domain.to_string(),
            policy: dmarc.p,
            record: None,
        };

        let has_dkim_pass = dkim_output.iter().any(|o| o.result == DkimResult::Pass);
        if spf_output.result == SpfResult::Pass || has_dkim_pass {
            // Check SPF alignment
            let from_subdomain = format!(".{}", from_domain);
            if spf_output.result == SpfResult::Pass {
                output.spf_result = if mail_from_domain == from_domain {
                    DmarcResult::Pass
                } else if dmarc.aspf == Alignment::Relaxed
                    && mail_from_domain.ends_with(&from_subdomain)
                    || from_domain.ends_with(&format!(".{}", mail_from_domain))
                {
                    output.policy = dmarc.sp;
                    DmarcResult::Pass
                } else {
                    DmarcResult::Fail(Error::DMARCNotAligned)
                };
            }

            // Check DKIM alignment
            if has_dkim_pass {
                output.dkim_result = if dkim_output.iter().any(|o| {
                    o.result == DkimResult::Pass && o.signature.as_ref().unwrap().d.eq(from_domain)
                }) {
                    DmarcResult::Pass
                } else if dmarc.adkim == Alignment::Relaxed
                    && dkim_output.iter().any(|o| {
                        o.result == DkimResult::Pass
                            && (o.signature.as_ref().unwrap().d.ends_with(&from_subdomain)
                                || from_domain
                                    .ends_with(&format!(".{}", o.signature.as_ref().unwrap().d)))
                    })
                {
                    output.policy = dmarc.sp;
                    DmarcResult::Pass
                } else {
                    if dkim_output.iter().any(|o| {
                        o.result == DkimResult::Pass
                            && (o.signature.as_ref().unwrap().d.ends_with(&from_subdomain)
                                || from_domain
                                    .ends_with(&format!(".{}", o.signature.as_ref().unwrap().d)))
                    }) {
                        output.policy = dmarc.sp;
                    }
                    DmarcResult::Fail(Error::DMARCNotAligned)
                };
            }
        }

        output.with_record(dmarc)
    }

    /// Validates the external report e-mail addresses of a DMARC record
    pub async fn verify_dmarc_report_address<'x>(
        &self,
        domain: &str,
        addresses: &'x [URI],
    ) -> Option<Vec<&'x URI>> {
        let mut result = Vec::with_capacity(addresses.len());
        for address in addresses {
            if address.uri.ends_with(domain)
                || match self
                    .txt_lookup::<Dmarc>(format!(
                        "{}.report.dmarc.{}.",
                        domain,
                        address
                            .uri
                            .rsplit_once('@')
                            .map(|(_, d)| d)
                            .unwrap_or_default()
                    ))
                    .await
                {
                    Ok(_) => true,
                    Err(Error::DNSError(_)) => return None,
                    _ => false,
                }
            {
                result.push(address);
            }
        }

        result.into()
    }

    async fn dmarc_tree_walk(&self, domain: &str) -> crate::Result<Option<Arc<Dmarc>>> {
        let labels = domain.split('.').collect::<Vec<_>>();
        let mut x = labels.len();
        if x == 1 {
            return Ok(None);
        }
        while x != 0 {
            // Build query domain
            let mut domain = String::with_capacity(domain.len() + 8);
            domain.push_str("_dmarc");
            for label in labels.iter().skip(labels.len() - x) {
                domain.push('.');
                domain.push_str(label);
            }
            domain.push('.');

            // Query DMARC
            match self.txt_lookup::<Dmarc>(domain).await {
                Ok(dmarc) => {
                    return Ok(Some(dmarc));
                }
                Err(Error::DNSRecordNotFound(_)) | Err(Error::InvalidRecordType) => (),
                Err(err) => return Err(err),
            }

            // If x < 5, remove the left-most (highest-numbered) label from the subject domain.
            // If x >= 5, remove the left-most (highest-numbered) labels from the subject
            // domain until 4 labels remain.
            if x < 5 {
                x -= 1;
            } else {
                x = 4;
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod test {
    use std::time::{Duration, Instant};

    use crate::{
        common::parse::TxtRecordParser,
        dkim::Signature,
        dmarc::{Dmarc, Policy, URI},
        AuthenticatedMessage, DkimOutput, DkimResult, DmarcResult, Error, Resolver, SpfOutput,
        SpfResult,
    };

    #[tokio::test]
    async fn dmarc_verify() {
        let resolver = Resolver::new_system_conf().unwrap();

        for (
            dmarc_dns,
            dmarc,
            message,
            mail_from_domain,
            signature_domain,
            dkim,
            spf,
            expect_dkim,
            expect_spf,
            policy,
        ) in [
            // Strict - Pass
            (
                "_dmarc.example.org.",
                concat!(
                    "v=DMARC1; p=reject; sp=quarantine; np=None; aspf=s; adkim=s; fo=1;",
                    "rua=mailto:dmarc-feedback@example.org"
                ),
                "From: hello@example.org\r\n\r\n",
                "example.org",
                "example.org",
                DkimResult::Pass,
                SpfResult::Pass,
                DmarcResult::Pass,
                DmarcResult::Pass,
                Policy::Reject,
            ),
            // Relaxed - Pass
            (
                "_dmarc.example.org.",
                concat!(
                    "v=DMARC1; p=reject; sp=quarantine; np=None; aspf=r; adkim=r; fo=1;",
                    "rua=mailto:dmarc-feedback@example.org"
                ),
                "From: hello@example.org\r\n\r\n",
                "subdomain.example.org",
                "subdomain.example.org",
                DkimResult::Pass,
                SpfResult::Pass,
                DmarcResult::Pass,
                DmarcResult::Pass,
                Policy::Quarantine,
            ),
            // Strict - Fail
            (
                "_dmarc.example.org.",
                concat!(
                    "v=DMARC1; p=reject; sp=quarantine; np=None; aspf=s; adkim=s; fo=1;",
                    "rua=mailto:dmarc-feedback@example.org"
                ),
                "From: hello@example.org\r\n\r\n",
                "subdomain.example.org",
                "subdomain.example.org",
                DkimResult::Pass,
                SpfResult::Pass,
                DmarcResult::Fail(Error::DMARCNotAligned),
                DmarcResult::Fail(Error::DMARCNotAligned),
                Policy::Quarantine,
            ),
            // Strict - Pass with tree walk
            (
                "_dmarc.example.org.",
                concat!(
                    "v=DMARC1; p=reject; sp=quarantine; np=None; aspf=s; adkim=s; fo=1;",
                    "rua=mailto:dmarc-feedback@example.org"
                ),
                "From: hello@a.b.c.example.org\r\n\r\n",
                "a.b.c.example.org",
                "a.b.c.example.org",
                DkimResult::Pass,
                SpfResult::Pass,
                DmarcResult::Pass,
                DmarcResult::Pass,
                Policy::Reject,
            ),
            // Relaxed - Pass with tree walk
            (
                "_dmarc.c.example.org.",
                concat!(
                    "v=DMARC1; p=reject; sp=quarantine; np=None; aspf=r; adkim=r; fo=1;",
                    "rua=mailto:dmarc-feedback@example.org"
                ),
                "From: hello@a.b.c.example.org\r\n\r\n",
                "example.org",
                "example.org",
                DkimResult::Pass,
                SpfResult::Pass,
                DmarcResult::Pass,
                DmarcResult::Pass,
                Policy::Quarantine,
            ),
            // Failed mechanisms
            (
                "_dmarc.example.org.",
                concat!(
                    "v=DMARC1; p=reject; sp=quarantine; np=None; aspf=s; adkim=s; fo=1;",
                    "rua=mailto:dmarc-feedback@example.org"
                ),
                "From: hello@example.org\r\n\r\n",
                "example.org",
                "example.org",
                DkimResult::Fail(Error::SignatureExpired),
                SpfResult::Fail,
                DmarcResult::None,
                DmarcResult::None,
                Policy::Reject,
            ),
        ] {
            resolver.txt_add(
                dmarc_dns,
                Dmarc::parse(dmarc.as_bytes()).unwrap(),
                Instant::now() + Duration::new(3200, 0),
            );

            let auth_message = AuthenticatedMessage::parse(message.as_bytes()).unwrap();
            let signature = Signature {
                d: signature_domain.into(),
                ..Default::default()
            };
            let dkim = DkimOutput {
                result: dkim,
                signature: (&signature).into(),
                report: None,
                is_atps: false,
            };
            let spf = SpfOutput {
                result: spf,
                domain: mail_from_domain.to_string(),
                report: None,
                explanation: None,
            };
            let result = resolver
                .verify_dmarc(&auth_message, &[dkim], mail_from_domain, &spf)
                .await;
            assert_eq!(result.dkim_result, expect_dkim);
            assert_eq!(result.spf_result, expect_spf);
            assert_eq!(result.policy, policy);
        }
    }

    #[tokio::test]
    async fn dmarc_verify_report_address() {
        let resolver = Resolver::new_system_conf().unwrap();
        resolver.txt_add(
            "example.org.report.dmarc.external.org.",
            Dmarc::parse(b"v=DMARC1").unwrap(),
            Instant::now() + Duration::new(3200, 0),
        );
        let uris = vec![
            URI::new("dmarc@example.org", 0),
            URI::new("dmarc@external.org", 0),
            URI::new("domain@other.org", 0),
        ];

        assert_eq!(
            resolver
                .verify_dmarc_report_address("example.org", &uris)
                .await
                .unwrap(),
            vec![
                &URI::new("dmarc@example.org", 0),
                &URI::new("dmarc@external.org", 0),
            ]
        );
    }
}
