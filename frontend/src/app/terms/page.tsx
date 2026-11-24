import type { Metadata } from "next";
import Link from "next/link";
import LegalLayout, { Contact, Section } from "@/components/LegalLayout";
import { dmcaAgent } from "@/lib/legal";

export const metadata: Metadata = { title: "Terms of Service — VectorScrub" };

export default function TermsPage() {
  const agent = dmcaAgent();
  return (
    <LegalLayout title="Terms of Service">
      <Section title="1. What VectorScrub is, and what it is not">
        <p>
          VectorScrub, a product of VectorGuard Labs (&quot;we&quot;), checks a set of data-broker and
          people-search websites for listings that match you, helps you ask those sites to remove them,
          and re-checks the listings afterwards. By creating an account you agree to these terms and to
          our <Link href="/privacy" className="text-brand-blue-400 underline">Privacy Policy</Link>.
        </p>
        <p>
          <strong className="text-white">We cannot remove you from every site, and we do not promise to.</strong>{" "}
          Many brokers block automated tools, require you to click a link sent to your own inbox, or
          can only be searched by hand. We do not get around those protections. Whether a site removes a
          listing is up to that site.
        </p>
        <p>
          We ask for removal in two different ways. <strong className="text-white">Individual opt-out
          requests</strong> go to a data-broker or people-search site for one listing at a time.{" "}
          <strong className="text-white">Authorized-agent privacy requests</strong> are a separate, limited
          feature: you choose companies, and we email each one a privacy request on your behalf. Section 4
          explains both. California&apos;s own Delete Request and Opt-out Platform (DROP) is a third thing, run
          by the State of California. We do not submit requests to it for you.
        </p>
      </Section>

      <Section title="2. This is an early pilot">
        <p>
          The service is new. Parts of it work well, parts do not, and some of it will change. During
          the pilot you should expect that:
        </p>
        <ul className="list-disc ml-5 space-y-1">
          <li>
            We can currently read only some of the sites we list. Your dashboard and the home page show
            which. Finding nothing on a site we could not read does not mean you are not listed there.
          </li>
          <li>
            For sites that block automation, you will be asked to complete the opt-out yourself, using
            the steps we show you. You then tell us you did it, and we check the listing ourselves.
          </li>
          <li>
            Some sites send a confirmation email to you. You must click it, or the request does not count.
          </li>
          <li>
            During the pilot you can ask a person to help with a removal we could not automate. It is
            free, comes with no guaranteed response time, and we will never ask for your email password
            or to get around a site&apos;s own verification.
          </li>
          <li>We may ask for your feedback. We count anonymized results (such as how many requests were verified), never your details. Being interviewed, or being asked about sharing an anonymized version of your results, are separate optional choices at sign-up and are not required.</li>
          <li>
            Authorized-agent privacy requests (section 4) are a limited pilot. They are offered only to some paid
            customers, for now only to residents of California, and we may change, pause or switch them off at any
            time. If the Deletion requests page in your dashboard does not offer them to you, they are not
            available to you yet.
          </li>
          <li>Features, limits and prices may change. We will not remove a feature you paid for without telling you first.</li>
        </ul>
      </Section>

      <Section title="3. Who can use VectorScrub, and what you must do">
        <ul className="list-disc ml-5 space-y-1">
          <li><strong className="text-white">You must be 18 or older.</strong> We ask you to confirm this when you create an account. If we learn that an account belongs to someone under 18, we will close it and erase its data.</li>
          <li>Use VectorScrub only for <strong className="text-white">your own</strong> information. You may not enter another person&apos;s details, or use the service to find or locate someone. What you authorize covers your own information only, never another person&apos;s, including a family member who asks you to.</li>
          <li>Give accurate details. A wrong name, address or date of birth leads to wrong matches and failed requests.</li>
          <li>Keep your password private. You are responsible for activity in your account.</li>
          <li>Complete any manual steps and confirmation emails we ask for, if you want those removals to happen.</li>
          <li>Tell us if a listing we report as removed is still there, or one we report as present is not you.</li>
        </ul>
      </Section>

      <Section title="4. Your authorization">
        <p>VectorScrub offers two ways to request removal of your personal information.</p>
        <p>
          <strong className="text-white">Individual opt-out requests.</strong> We submit a removal request to a
          site only after you press the authorize button for that specific listing. By pressing it you authorize
          us to send the opt-out request to that site on your behalf, using the details you have given us that
          the site requires.
        </p>
        <p>
          <strong className="text-white">Authorized-agent privacy requests.</strong> Where available, you may select
          companies and authorize VectorScrub, a product of VectorGuard Labs, to act as your authorized agent in
          sending privacy requests to them on your behalf. A request may ask a company to delete the personal
          information it holds about you and, where the company&apos;s page offers it, to stop selling or sharing it.
          You choose each of those two kinds separately. Before anything is sent you must review the companies, the
          kinds of request and the authorization text on the page, and tick that you authorize them. That
          authorization covers only the companies and the kinds of request you selected for that campaign.
        </p>
        <p>
          You authorize us to send each selected company your name and email address. We send your street address
          only if you tick the separate box to include it, or if the company&apos;s page says it needs the address to
          find your records (the page tells you before you submit). We never send your phone numbers or date of
          birth. The email goes out from our own email service, and replies go to you, not to us.
        </p>
        <p>
          Some companies may require more information, identity verification, direct confirmation from you, or
          signed proof of your authorization. You may need to complete those steps with the company yourself. A
          company may decline a request, and we cannot promise that any company will accept or act on one, or
          answer within any set time. We do not read the company&apos;s replies.
        </p>
        <p>
          <strong className="text-white">Signing.</strong> You sign each campaign electronically: you agree to sign
          electronically and type your full name, which must match the name in your saved details because it is the
          name the companies receive. Your typed name is your signature on that authorization. We keep the signed copy
          and you can download it as a PDF from the Deletion requests page, to give to a company that asks for proof
          of your signed permission. Each email tells the company the copy exists and that it can ask you for it.
          Whether a company accepts an electronic signature is the company&apos;s decision.
        </p>
        <p>
          You may withdraw a request that has not been sent yet, with the Withdraw button on the Deletion requests
          page or by writing to us. Withdrawing a whole campaign also withdraws the authorization itself: nothing still
          waiting under it is sent afterwards, and we record when you withdrew. Withdrawing cannot recall an email
          that has already been sent.
        </p>
        <p>
          If sending is interrupted, we do not send again on our own, because the email may already have left. A
          person checks our email provider&apos;s records for that message. If it left, we record it as sent; if it did
          not, we send it once. Nothing is sent again under an authorization you withdrew.
        </p>
        <p>
          We keep a record of what you authorized and what we sent, which you can see on the same page, for
          accountability and security. Our role is to send and track requests. We do not guarantee deletion and we
          do not control how a company handles your information. The{" "}
          <Link href="/privacy" className="text-brand-blue-400 underline">Privacy Policy</Link> (section 7) says what
          is kept if you delete your account.
        </p>
        <p>
          <strong className="text-white">California DROP.</strong> California&apos;s Delete Request and Opt-out Platform
          is a separate, free state service with its own eligibility, identity checks and rules for agents. It sends
          one request to every registered data broker. VectorScrub does not submit DROP requests for you today, and
          our requests do not replace DROP: we suggest California residents use both. The dashboard links to the
          state&apos;s own site. If we ever offer help with DROP, we will change these terms and ask you first.
        </p>
        <p>
          If a removed listing reappears, we alert you. We do <strong className="text-white">not</strong>{" "}
          submit a new request on our own: you authorize it again.
        </p>
        <p>
          Some sites ask for a copy of an ID or other extra proof. We will not submit that for you
          without asking you first.
        </p>
      </Section>

      <Section title="5. How we decide a listing is &quot;removed&quot;">
        <p>
          We mark a listing removed only when our own re-check finds positive evidence: the page is gone,
          or it loads and no longer shows you. A page we cannot load, a block, a CAPTCHA, or an
          unclear result is reported as <em>inconclusive</em>, never as removed. Telling us you did an
          opt-out by hand is recorded as what you reported; it does not mark anything removed. We
          re-check on a schedule, about 3, 7, 14 and 30 days after you submitted it. Sites vary and some
          take weeks. If a request cannot be
          confirmed within 45 days, it is closed as unverifiable and we tell you.
        </p>
        <p>
          <strong className="text-white">Authorized-agent requests are different.</strong> For these, &quot;sent&quot;
          means we emailed the request to the company. We do not read the reply and we run no check of our own, so
          we never mark one as removed ourselves. &quot;Company says it deleted your data&quot; is what you tell us you
          heard. It is recorded as your report, not as a verified removal. If no answer has been reported after
          45 days we say so, which does not mean the company is late: it may have replied to your email, or taken
          the extra time the law can allow.
        </p>
        <p>
          Our checks can be wrong. A listing may exist somewhere we do not look, or may return later.
        </p>
      </Section>

      <Section title="6. Plans and payment" id="plans">
        <ul className="list-disc ml-5 space-y-1">
          <li><strong className="text-white">Free:</strong> a scan every 7 days across up to 4 sites and up to 3 opt-out requests (requests that fail do not count). Free accounts are not monitored.</li>
          <li>
            <strong className="text-white">Founding pilot:</strong> a one-time payment of $1 for a small group of
            early customers. It does not renew and starts no subscription. It includes what is on the{" "}
            <Link href="/pilot" className="text-brand-blue-400 underline">pilot page</Link>. It guarantees no removal. If the
            pilot is full or ends, we will say so.
          </li>
          <li>
            <strong className="text-white">Paid plans:</strong> we are not yet selling a subscription unless the billing
            page of your account says so. Before you are charged for anything that renews, you will see the
            price and what it includes, and you must choose it. If you buy the Early Access plan ($4.99 per
            month, billed through Stripe), you keep that price while you stay subscribed, even if we raise it for
            new members, and you can cancel at any time from the billing page. Cancelling stops future charges
            and your plan returns to Free at the end of the period you paid for. We do not refund the current
            period, except where the law requires it or we say otherwise in writing.
          </li>
          <li>
            <strong className="text-white">Authorized-agent privacy requests</strong> (section 4) are part of the paid
            plans, not Free, and cost nothing extra during the pilot. We do not send the same company a second request
            from you within 90 days. If your plan returns to Free before a waiting request is sent, it is not sent.
          </li>
          <li>We do not store your card number. Stripe handles payment.</li>
        </ul>
        <p>
          Because the service cannot reach every site, what you pay buys the work we do and the tracking we
          provide, not a guaranteed outcome. If you think the service did not do what this page says, write to
          us and we will make it right or refund you.
        </p>
      </Section>

      <Section title="7. Ending your account and your data">
        <p>
          You can stop using the service at any time. You can delete your account yourself
          from your profile page, which erases your personal details at once, or write to <Contact /> and we
          will do it for you. Cancel any paid plan first. What we keep afterwards is described in the <Link href="/privacy" className="text-brand-blue-400 underline">Privacy Policy</Link>.
        </p>
        <p>
          Deleting your account also deletes your authorized-agent campaigns and requests, including the
          authorization text we kept. It cannot recall emails already sent: the companies keep their copies, and
          our email provider may keep one too. Privacy Policy section 7 lists exactly what stays.
        </p>
        <p>
          We may suspend or close an account that breaks these terms, for example by searching for
          someone else, abusing the service, or attempting to attack it. We will explain why when we can.
        </p>
      </Section>

      <Section title="8. Limits on our responsibility">
        <p>
          The service is provided as it is during the pilot. To the extent the law allows, we are not
          liable for sites that do not honor a request, for listings that return, or for indirect losses.
          Our total liability for any claim is limited to the amount you paid us in the 12 months before
          it. Nothing in these terms limits rights you have that the law does not allow us to limit.
        </p>
        <p>
          When we send a request as your authorized agent, we act only on your instructions. We are not
          responsible for a company&apos;s decision, for what it does with your information, or for a company that
          does not accept a request from an agent.
        </p>
        <p>
          VectorScrub is not a law firm and does not give legal advice. It is not affiliated with any
          data broker or with any government service.
        </p>
      </Section>

      <Section title="9. Copyright">
        <p>
          VectorScrub does not publish anything its customers enter. Your details are kept private to
          your account, so the service does not host public user content. We do not copy broker
          listings except for the short text we need to decide whether a page is about you, and we do
          not republish it.
        </p>
        {agent ? (
          <>
            <p>
              If you believe material in the service infringes your copyright, send a notice that meets
              17 U.S.C. 512(c)(3) to our designated agent (registered with the U.S. Copyright Office as{" "}
              {agent.registration}):
            </p>
            <p className="text-white">
              {agent.name}
              <br />
              {agent.address}
              <br />
              <a href={`mailto:${agent.email}`} className="text-brand-blue-400 underline">{agent.email}</a>
            </p>
          </>
        ) : (
          <p>
            If you believe material in the service infringes your copyright, write to <Contact /> with
            the work, where you found it in VectorScrub, and how to reach you.
          </p>
        )}
      </Section>

      <Section title="10. Changes and contact">
        <p>
          If we change these terms in a way that matters, we will tell you before the change applies
          to you, and you may cancel if you do not agree. Questions, problems, and data requests: <Contact />.
        </p>
      </Section>
    </LegalLayout>
  );
}
