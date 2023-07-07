import DbTable from './Contestant.js'
import { friendlyProblem, friendlyFix } from './utils.js'
import { ref, computed } from 'vue'

function fullName(first, last) {
  if (!first) { return last ?? undefined; }
  if (!last) { return first ?? undefined; }
  return `${first} ${last}`
}

function dbContestantCat(value) {
  if (!value) { return }
  return value === "M" ? "Cowboys" : ("F" ? "Cowgirls" : "Unknown")
}

const DataCell = {
  props: {
    value: Object | String,
  },
  setup(props) {
    return {
    }
  },
  template: `
  <td class="text-center"
    v-if="value !== undefined && value !== null">{{value}}</td>
  <td v-else class="text-center">-</td>
  `
}

const AddressCell = {
  props: {
    line1: String | null,
    line2: String | null,
    city: String,
    region: String,
    country: String | null,
    postalCode: String,
  },
  setup(props) {
    return {
    }
  },
  template: `
  <td class="text-center oldstyle-nums">
      {{line1}}
      <template v-if="line2"><br>{{line2}}</template>
      <br>{{city}}, {{region}} {{postalCode}}
      <template v-if='country && country !== "United States"'><br>{{country}}</template>
  </td>
  `
}

const DbRecordRow = {
  components: {
    DataCell,
    AddressCell,
  },
  props: {
    header: String,
    record: Object | null,
  },
  setup(props) {
    return {
      fullName,
      dbContestantCat,
    }
  },
  template: `
    <tr>
      <th>{{header}}</th>
      <data-cell :value=record?.igra_number></data-cell>
      <data-cell :value=record?.association></data-cell>
      <data-cell :value="fullName(record?.legal_first, record?.legal_last)"></data-cell>
      <data-cell :value="fullName(record?.first_name, record?.last_name)"></data-cell>
      <data-cell :value=record?.birthdate></data-cell>
      <data-cell :value=dbContestantCat(record?.sex)></data-cell>

      <address-cell v-if="record"
        :line1=record.address
        :city=record.city
        :region=record.state
        :postalCode=record.zip
      ></address-cell>
      <data-cell v-else></data-cell>

      <data-cell :value=record?.email></data-cell>
      <data-cell :value=record?.cell_phone></data-cell>
      <data-cell :value=record?.home_phone></data-cell>
      <data-cell></data-cell>
    </tr>
  `
}

const EventRow = {
  props: {
    event: Object,
    issues: Array,
    relevant: Object,
    partners: Array,
  },
  setup(props) {
    const theseIssues = computed(() => {
      let { issues, event } = props;
      if (!issues || !event) { return []; }

      return issues.filter((issue) => {
          return (
            issue.problem.data && issue.problem.data.event === event.rodeoEventRelId
              && (
                !issue.problem.data.round
                || issue.problem.data.round === event.round
              )
          )
      })
    })

    const bgColor = computed(() => {
      return theseIssues.value.length ? "bg-red-400" : "#f9f9f9"
    })

    function problemName(issue) {
      return friendlyProblem(issue)
    }

    function suggestedFix(issue) {
      if (issue.fix.name === "UseThisRecord") {
        let other = props.relevant[issue.fix.data];
        if (other) {
          return `Maybe they meant ${other.first_name} ${other.last_name} with ID ${other.igra_number}?`
        }
      }
      return friendlyFix(issue)
    }

    const partnerInfo = computed(() => {
      let { event, partners, relevant } = props
      if (!event.partners || !partners) {
        return []
      }

      let eventPartners = partners.filter((p) => {
        return p.event === event.rodeoEventRelId && p.round === event.round
      });

      // match the user value to the correct partner.
      return event.partners.map((nameOrNum) => {
        let lowerNN = nameOrNum.trim().toLowerCase()
        if (lowerNN === "") { return {} }

        // among their known partners, find a match
        // if the lowerNN contains the IGRA number or partner name
        return eventPartners.find((p) => {
          if (lowerNN.includes(p.igra_number)) { return true }

          let dbPartner = relevant[p.igra_number]
          if (!dbPartner) { return false }

          let partnerName = `${dbPartner.first_name} ${dbPartner.last_name}`.toLowerCase()
          let partnerLegalName = `${dbPartner.legal_first} ${dbPartner.legal_last}`.toLowerCase()
          return lowerNN.includes(partnerName) || lowerNN.includes(partnerLegalName)
        })
      }).map((p) => { return p ? relevant[p.igra_number] : undefined })
    })

    function formatPartner(p) {
      if (!p) { return }
      return `${p.igra_number} ${p.first_name} ${p.last_name}`
    }

    return {
      problemName,
      suggestedFix,
      theseIssues,
      bgColor,
      partnerInfo,
      formatPartner,
    }
  },
  template: `
      <tr :class="bgColor" class="text-center">
        <td>{{event.rodeoEventRelId}}</td>
        <td>{{event.round}}</td>
        <td>{{event.partners[0]}}</td>
        <td>{{event.partners[1]}}</td>
        <td>{{event.partners[2]}}</td>
      </tr>
      <tr :class="bgColor" class="text-left text-sm" v-for="issue in theseIssues">
        <td colspan="5" class="">
        → {{problemName(issue)}}
        {{suggestedFix(issue)}}
        </td>
      </tr>
      <tr>
        <td></td>
        <td></td>
        <td v-if="partnerInfo[0]">Match: {{formatPartner(partnerInfo[0])}}</td>
        <td v-else></td>
        <td v-if="partnerInfo[1]">Match: {{formatPartner(partnerInfo[1])}}</td>
        <td v-else></td>
        <td v-if="partnerInfo[2]">Match: {{formatPartner(partnerInfo[2])}}</td>
        <td v-else></td>
      </tr>
      <tr>
        <td></td>
        <td></td>
        <td></td>
        <td></td>
        <td></td>
      </tr>
  `
};


export default {
  components: {
    DbTable,
    EventRow,
    DataCell,
    AddressCell,
    DbRecordRow,
  },
  props: {
    contestant: Object,
    partners: Array,
    found: Object,
    events: Array,
    issues: Array,
    relevant: Object,
  },
  setup(props) {

    const contestantBday = computed(() => {
      let { contestant } = props;
      return `${contestant.dob.month}/${contestant.dob.day}/${contestant.dob.year}`
    })

    // return true if the given issue's problem matches any of the given fields
    function isThisField(issue, fields) {
      return compareTo.some((f) => issue.problem.data.field === f)
    }

    // fields the registrant didn't fill in
    const unfilledFields = computed(() => {
      let { issues } = props;
      if (!issues) { return false; }

      return (issues
        .filter((issue) => issue.problem.name === "NoValue")
        .map((issue) => issue.problem.data.field)
      )
    })

    // Do we have a database value, but different from this one?
    const isDbMismatch = computed(() => {
      let { issues, field, fields } = props;
      if (!issues) { return false; }
      let compareTo = field ? [field] : (fields ?? []);

      return issues.some((issue) => {
          return (
            issue.problem.name === "DbMismatch"
            && compareTo.some((f) => issue.problem.data.field === f)
          )
      })
    })

    // Should we expect _any_ DB data?
    const missingDbValue = computed(() => {
      let { issues } = props;
      if (!issues) { return false; }
      return issues.some((issue) => {
        return (
          issue.problem.name === "NotAMember"
          || issue.problem.name === "MaybeAMember"
          || issue.problem.name === "NoPerfectMatch"
        )
      });
    })

    // Possible matches to the registrant
    const maybeMatches = computed(() => {
      let { issues, relevant } = props
      if (!issues || !relevant) { return [] }

      return (
        issues
          .filter((issue) => issue.problem.name === "NoPerfectMatch" && issue.fix.name === "UseThisRecord")
          .map((issue) => relevant[issue.fix.data])
          .filter((record) => record !== undefined)
      )
    })

    return {
      dbContestantCat,
      fullName,
      contestantBday,
      maybeMatches,
    }
  },
  template: `
  <article>
    <div class="">
    <section>
      <header class="text-lg">Registration Data</header>

      <table class="table-auto">
        <thead>
          <tr>
            <th></th>
            <th>IGRA Number</th>
            <th>Association</th>
            <th>Legal Name</th>
            <th>Performance Name</th>
            <th>Date of Birth</th>
            <th>Competes With</th>
            <th>Address</th>
            <th>Email</th>
            <th>Cell Phone</th>
            <th>Home Phone</th>
            <th>Note to Director</th>
          </tr>
        </thead>
        <tbody>

          <tr>
            <th>Registration Data</th>
            <data-cell :value=contestant.association?.igra></data-cell>
            <data-cell :value=contestant.association?.memberAssn></data-cell>
            <data-cell :value="fullName(contestant.firstName, contestant.lastName)"></data-cell>
            <data-cell :value=contestant.performanceName></data-cell>
            <data-cell :value=contestantBday></data-cell>
            <data-cell :value=contestant.gender></data-cell>

            <address-cell
              :line1=contestant.address.addressLine1
              :line2=contestant.address.addressLine2
              :city=contestant.address.city
              :region=contestant.address.region
              :country=contestant.address.country
              :postalCode=contestant.address.zipCode
            ></address-cell>

            <data-cell :value=contestant.address.email></data-cell>
            <data-cell :value=contestant.address.cellPhoneNo></data-cell>
            <data-cell :value=contestant.address.homePhoneNo></data-cell>
            <data-cell :value=contestant.noteToDirector></data-cell>
          </tr>

          <db-record-row header="Database Record"
            :class="{ 'bg-yellow-200': !found }"
            :record=found></db-record-row>
          <db-record-row v-for="match in maybeMatches" header="Possible Match"
            class="bg-blue-200"
            :record=match></db-record-row>

        </tbody>
      </table>
    </section>

    </div>


    <section>
      <header class="text-lg">Registered Events</header>
      <table class="table-auto">
        <thead>
          <tr>
            <th>Event</th>
            <th>Round</th>
            <th>Partner 1</th>
            <th>Partner 2</th>
            <th>Partner 3</th>
          </tr>
        </thead>
        <tbody>
          <event-row v-for="event in events"
            :event=event
            :issues=issues
            :relevant=relevant
            :partners=partners
          ></event-row>
        </tbody>
      </table>
    </section>


    <section v-if="issues">
      <header class="text-lg">Problems</header>
      <table class="table-auto">
        <thead>
          <tr>
            <th>Problem</th>
            <th></th>
            <th>Suggestion</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="issue in issues">
            <td>{{issue.problem.name}}</td>
            <td><template v-if="issue.problem.data">{{issue.problem.data}}</template></td>
            <td>{{issue.fix.name}}</td>
            <td><template v-if="issue.fix.data">{{issue.fix.data}}</template></td>
          </tr>
        </tbody>
      </table>
    </section>

  </article>
  `
}
