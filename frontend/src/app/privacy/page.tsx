import type { Metadata } from "next";
import Link from "next/link";
import LegalLayout, { Contact, Section } from "@/components/LegalLayout";

export const metadata: Metadata = { title: "Privacy Policy — VectorScrub" };

export default function PrivacyPage() {
  return (
    <LegalLayout title="Privacy Policy">
      <Section title="1. The short version">
        <ul className="list-disc ml-5 space-y-1">
          <li>We collect only what is needed to find and remove your listings.</li>
          <li>Your identity details are encrypted. We never sell them, never use them to train AI models, and never share them for advertising.</li>
          <li>You can delete your account yourself, and it erases your data at once. Section 7 says exactly what is kept.</li>
          <li>Removing you from a broker means telling that broker who you are. That is unavoidable, and we only do it when you authorize it.</li>
          <li>If you use the authorized-agent feature, we email the companies you choose, as your agent, with your name and email address. We add your street address only if you separately agree or the company needs it to find you. Once an email is sent we cannot take it back.</li>
        </ul>
      </Section>

      <Section title="2. What we collect">
        <ul className="list-disc ml-5 space-y-1">
          <li><strong className="text-white">Account:</strong> your email address and a password (stored only as a salted hash).</li>
          <li><strong className="text-white">Identity details you enter:</strong> for example your name, addresses, phone numbers, email addresses and date of birth. These are what we search for and what opt-out forms ask for.</li>
          <li><strong className="text-white">What we find:</strong> the listings that match you (site, link, what kind of information it shows, and a severity score), plus scan history and your privacy score. <strong className="text-white">The link to a listing can contain your name</strong> (some sites put it in the address). Links you add yourself, to listings you found, are handled the same way. We encrypt those links in our database with your own key, like your identity details, and open them only to show them to you or to the scanning software that has to visit them.</li>
          <li><strong className="text-white">Your actions:</strong> removal authorizations, request status, the outcome of each attempt, what you tell us about opt-outs you did by hand (the date, and whether you confirmed the site&apos;s email), whether you asked for help, and a record of when your details were used.</li>
          <li>
            <strong className="text-white">Authorized-agent request records:</strong> for each campaign, the exact
            authorization text you accepted with its version and SHA-256 hash, the companies and kinds of request you
            chose, whether you agreed to include your address, the times, and for each company whether the request was
            sent, withdrawn or failed (with a short reason) and what you tell us the company answered. They also hold the{" "}
            <strong className="text-white">signed authorization</strong>: a document with your name, the email address the
            requests use, the agent, what each company is asked, the time and your typed signature, encrypted with your own
            key, and the SHA-256 fingerprint of its text. Apart from that signed copy, these records hold no name, email or
            address of yours; those are read from your encrypted details only when an email is sent. For each email we also
            record the technical message identifier it was sent with, so an interrupted send can be traced.
          </li>
          <li><strong className="text-white">Billing:</strong> your plan, and Stripe customer and subscription identifiers. We do not receive your card number.</li>
          <li><strong className="text-white">Technical:</strong> IP address and browser type in security logs, and a session cookie (see section 8).</li>
        </ul>
      </Section>

      <Section title="3. How we use it">
        <ul className="list-disc ml-5 space-y-1">
          <li>To scan broker sites for listings that match you, and to judge whether a page really is about you.</li>
          <li>To submit opt-out requests you have authorized, and to re-check listings afterwards.</li>
          <li>
            <strong className="text-white">Authorized-agent privacy requests:</strong> when you select and authorize
            companies for a campaign, we act as your authorized agent to send the requests you selected. We use your
            authorization and the personal information needed to prepare and send each request. We do not send requests
            to companies you have not selected and authorized.
          </li>
          <li>
            We use authorization records only to carry out your requests, verify that you authorized them, prevent
            fraud, keep an accurate account of what we did, and deal with disputes or legal obligations.
          </li>
          <li>To alert you, run your account, take payment, prevent abuse, and keep a security record.</li>
          <li>To improve the service using counts and outcomes (for example how many sites we could read, or how many requests were verified), not your identity details.</li>
          <li>If you ticked the optional boxes at sign-up, to contact you about a follow-up interview, or to ask whether we may share an anonymized version of your results. Ticking is not permission to publish: we would ask you first, and you can say no.</li>
        </ul>
        <p>
          We do not sell your information, share it for advertising, or use it to train AI models, ours
          or anyone&apos;s.
        </p>
      </Section>

      <Section title="4. Who else sees it">
        <ul className="list-disc ml-5 space-y-1">
          <li>
            <strong className="text-white">The sites you authorize for an individual opt-out.</strong> To remove a
            listing, the opt-out form or request typically needs your name and often your email or the
            link to your listing. We send only what that site requires, and only for listings you authorized.
          </li>
          <li>
            <strong className="text-white">The companies you select for an authorized-agent request.</strong> We act
            as your authorized agent and send each selected company an email from VectorScrub&apos;s email service. It
            contains your name and email address, the kinds of request you chose, and the authorization you accepted
            (its text, date and version) with the fingerprint of your signed copy, so the company can see what it is
            relying on and ask you for the signed copy. Your street address is
            included only if you ticked the separate box, or if the company&apos;s page says it needs it to find your
            records. We never send your phone numbers or date of birth. A company may contact you directly to verify
            your identity or to confirm that you authorized this, and some may require extra information or signed
            authorization. The dashboard shows each company, the address we would email and the page that address
            comes from, before you approve.
          </li>
          <li><strong className="text-white">Our email provider,</strong> which carries those emails and may keep copies of them.</li>
          <li>
            <strong className="text-white">An AI provider, to help read pages.</strong> To decide whether
            a broker page is about you, we send the page text to a large language model provider. Before
            that, we remove your own details and other people&apos;s names and contact details from the text,
            we do not send your identity or the page link, and we stop the request if any of your details
            remain. This is enforced in our code. The provider is Anthropic.
          </li>
          <li><strong className="text-white">Stripe,</strong> to take payment and manage subscriptions.</li>
          <li><strong className="text-white">Our hosting and infrastructure providers,</strong> who store the encrypted data and run the service for us.</li>
          <li><strong className="text-white">Anyone the law requires us to tell,</strong> if we receive a valid legal demand.</li>
        </ul>
        <p>
          We do not read replies that companies send to your inbox. We record what we submitted and any outcome you
          report to us.
        </p>
        <p>
          <strong className="text-white">California DROP.</strong> California&apos;s Delete Request and Opt-out Platform
          (DROP) is a separate, free state service with its own eligibility, identity-verification and authorization
          rules. VectorScrub does not submit DROP requests for you today, and shares nothing with DROP. Our requests do
          not replace it. The dashboard links California residents to the state&apos;s own site. If we ever offer help
          with DROP, we will update this policy first.
        </p>
      </Section>

      <Section title="5. How we protect it">
        <ul className="list-disc ml-5 space-y-1">
          <li>Your identity details are encrypted in our database with AES-256-GCM, using a separate key for each customer derived from a master key held only by our backend service.</li>
          <li>Our scanning and removal software does not hold that master key. It receives the minimum details it needs for one task, and each release of your details is recorded.</li>
          <li>The audit log of actions on your account cannot be edited or deleted after it is written.</li>
          <li>The signed authorization is encrypted with your own key, like your identity details, and only you can open it, from your account. The other authorization records hold no identity details. The details for an email are read from your encrypted profile only at the moment it is sent, only the fields you agreed to share, and each release is recorded.</li>
          <li>Passwords are hashed. Sessions use a cookie that scripts on the page cannot read.</li>
        </ul>
        <p>
          No system is perfectly secure. If we find that your information was exposed, we will tell you
          and say what we know. We do not currently hold a security certification such as SOC 2.
        </p>
      </Section>

      <Section title="6. What we cannot see or control">
        <p>
          Brokers keep their own copies and may share them with others. Removing a listing from one site
          does not remove it from sites we do not cover, and sites can republish it. We also do not read
          your inbox: if a site emails you a confirmation link, you click it.
        </p>
        <p>
          For authorized-agent requests, a company keeps its own copy of the email it receives and decides what to do
          with it. It may write to you directly to check who you are, ask for more, ask for signed authorization, or
          decline. Its replies go to your inbox, not to us.
        </p>
      </Section>

      <Section title="7. How long we keep it, and deleting it">
        <p>
          We keep your account, identity details, scan results, removal requests, authorized-agent campaign
          records and alerts for as long as your account exists. Privacy-score history older than one year is deleted automatically.
          We do not yet automatically delete inactive accounts.
        </p>
        <p>
          <strong className="text-white">Authorized-agent request records.</strong> While your account is active we
          keep, for each campaign, the authorization text and its SHA-256 hash, the version, your signed copy (encrypted)
          and its fingerprint, the companies and kinds of request you chose, the times (including when you withdrew),
          and the status of each request (sent, withdrawn or failed, with any outcome you reported). We do not delete
          these automatically, so that you and we can show what you authorized if a company or a dispute asks.
        </p>
        <p>
          Our security log records sign-ins, failed sign-ins and account changes. After 180 days the
          email address, IP address and browser details are removed from each entry, whether or not an
          account exists for it (a mistyped or guessed email address does not stay on file). The
          entry itself, what happened and when, is kept.
        </p>
        <p>
          <strong className="text-white">You can delete your account yourself</strong> from your profile
          page (enter your password to confirm). This permanently erases your account, identity
          details, scans, exposures, removal requests, authorized-agent campaigns and requests (including the
          authorization text, its hash and your signed copy), alerts and settings, at once. You can also write
          to <Contact /> and we will do it for you.
        </p>
        <p>What we keep afterwards:</p>
        <ul className="list-disc ml-5 space-y-1">
          <li>
            <strong className="text-white">A security record of what happened.</strong> Our audit logs
            cannot be rewritten, so they stay, but your name, email address, IP address, browser details
            and the free-text notes on removal attempts are removed from them first. What remains says,
            for example, that a removal attempt on some site failed on some date, not whose it was. For
            authorized-agent requests, the entries say that an authorization, a send or a withdrawal happened, when,
            and which companies (by our internal name) and kinds of request were involved. Once your account is deleted
            nothing links them to you.
          </li>
          <li>
            <strong className="text-white">Emails already sent.</strong> Deleting your account cannot recall them.
            The company keeps its copy, and our email provider may keep one in its sent records for as long as its own
            retention allows. They contain your name and email address, and your street address if you agreed to
            include it.
          </li>
          <li>
            <strong className="text-white">Payment identifiers.</strong> Stripe holds your payment
            records and may be required to keep them. We keep Stripe&apos;s reference numbers for the
            subscription, cut off from your account, for accounting and disputes.
          </li>
          <li>
            <strong className="text-white">Backups,</strong> for up to 30 days, encrypted. A backup
            made before you deleted your account still contains it until it expires. If we ever have
            to restore from one, we erase again every account that was deleted after it was made.
          </li>
        </ul>
        <p>
          If the law requires us to keep something, we keep only that, for as long as it requires.
        </p>
        <p>
          We keep what you give us only to provide the service described here, and nothing about you beyond the
          records listed in this section. We never receive the companies&apos; replies, so we keep none of them. A copy
          of the signed authorization you download is yours: deleting your account does not reach it.
        </p>
        <p>
          Deleting your account does not remove your listings from broker sites. If you have a paid
          plan, cancel it first on the billing page; we cannot delete an account whose subscription would
          keep charging you (one already set to end is fine).
        </p>
      </Section>

      <Section title="8. Cookies">
        <p>
          We use one session cookie to keep you signed in, and a security token that protects form
          submissions. We do not use advertising or analytics cookies, session recording or replay tools, or
          third-party trackers, and the site loads no fonts, scripts or images from other companies. Your
          browser talks only to us.
        </p>
      </Section>

      <Section title="9. Your rights">
        <p>
          Depending on where you live (for example California or the EU/UK) you may have the right to
          access, correct, delete, or receive a copy of your information, and to object to some uses. Where a law
          gives you a right, that law sets its scope and deadlines. Apart from that, as our own policy, we offer
          access, correction, deletion and a copy to every customer, wherever you live. Write to <Contact />. We
          will not treat you worse for asking.
        </p>
        <p>
          You also control authorized-agent requests: you choose the companies and the kinds of request, you can see
          the record of what you authorized, and you can withdraw any request that has not been sent yet from the
          Deletion requests page. A request already sent cannot be recalled.
        </p>
      </Section>

      <Section title="10. Children, changes, contact">
        <p>
          VectorScrub is for adults and is not directed at anyone under 18. We ask you to confirm that you
          are 18 or older when you create an account, and we do not knowingly collect information from
          anyone younger. If you think a child has given us information, write to <Contact /> and we will
          erase it. If we change this policy in a
          way that matters, we will tell you before the change applies to you. Read it together with the{" "}
          <Link href="/terms" className="text-brand-blue-400 underline">Terms of Service</Link>. Contact: <Contact />.
        </p>
      </Section>
    </LegalLayout>
  );
}
